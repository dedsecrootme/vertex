param(
    [ValidateSet('Plan','Start','Run','Status','Restore')][string]$Action='Status',
    [string]$Until
)
$ErrorActionPreference='Stop'
$project=Get-Item -LiteralPath (Split-Path -Parent $PSScriptRoot)
$resolved=$project.ResolveLinkTarget($true)
$root=if($resolved){$resolved.FullName}else{$project.FullName}
$dir=Join-Path $root 'database'
$binary=Join-Path $root 'target/release/crawler-rs.exe'
$statePath=Join-Path $dir 'night-state.json'
$drainState=Join-Path $dir 'drain-process.json'
$drainStop=Join-Path $dir 'drain.stop'
$nightStop=Join-Path $dir 'night.stop'
$logPath=Join-Path $dir 'night-supervisor.log'
$scriptPath=$PSCommandPath
$shell=(Get-Command pwsh).Source
$maxStorageMiB=81920
$minFreeMiB=51200
$serviceNames=@('CortexLauncherService','RGS','Razer Update Service','Razer Game Manager Service 3','Razer Chroma SDK Diagnostic Service','Razer Chroma SDK Server','Razer Chroma SDK Service','WYVRN Service','LGHUBUpdaterService','WSearch')

function Log([string]$message){
    [IO.File]::AppendAllText($logPath,"$([DateTimeOffset]::Now.ToString('o')) $message`r`n")
}
function Write-State {
    $temp="$statePath.$PID.tmp"
    [IO.File]::WriteAllText($temp,($script:state|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temp -Destination $statePath -Force
}
function Read-State {
    if(Test-Path -LiteralPath $statePath){return Get-Content -LiteralPath $statePath -Raw|ConvertFrom-Json -AsHashtable}
    return $null
}
function Get-Crawler {
    return @(Get-CimInstance Win32_Process -Filter "Name='crawler-rs.exe'" | Where-Object {$_.ExecutablePath -eq $binary -and $_.CommandLine.Contains($dir)})
}
function Get-Budget {
    $disk=[IO.DriveInfo]::new([IO.Path]::GetPathRoot($dir))
    $bytes=0L
    foreach($name in @('vertex.db','vertex.db-wal')){
        $path=Join-Path $dir $name
        if(Test-Path -LiteralPath $path){$bytes+=(Get-Item -LiteralPath $path).Length}
    }
    $os=Get-CimInstance Win32_OperatingSystem
    return @{freeMiB=[math]::Floor($disk.AvailableFreeSpace/1MB);storageMiB=[math]::Ceiling($bytes/1MB);availableMemoryMiB=[math]::Floor($os.FreePhysicalMemory/1024)}
}
function Stop-Crawler {
    [IO.File]::WriteAllText($drainStop,'graceful stop')
    $until=[DateTime]::UtcNow.AddSeconds(150)
    while((Get-Crawler).Count -gt 0){
        if([DateTime]::UtcNow -gt $until){throw 'Crawler did not stop gracefully; no force-kill and no second crawler launched.'}
        Start-Sleep -Seconds 1
    }
}
function Start-Crawler([int]$workers){
    if((Get-Crawler).Count -gt 0){throw 'A project crawler is already running.'}
    $budget=Get-Budget
    if($budget.freeMiB -lt $minFreeMiB -or $budget.storageMiB -ge $maxStorageMiB){throw 'Storage safety limit reached.'}
    if(Test-Path -LiteralPath $drainStop){Remove-Item -LiteralPath $drainStop}
    $stamp=Get-Date -Format 'yyyyMMdd-HHmmss'
    $log=Join-Path $dir "drain-$stamp.log"
    $errors=Join-Path $dir "drain-$stamp.err.log"
    $arguments=@('--drain','--db-dir',"`"$dir`"",'--workers',"$workers",'--session-secs','300','--idle-secs','1','--max-storage-mib',"$maxStorageMiB",'--min-free-mib',"$minFreeMiB",'--stop-file',"`"$drainStop`"")
    $process=Start-Process -FilePath $binary -ArgumentList $arguments -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError $errors -PassThru
    $record=@{pid=$process.Id;database=$dir;log=$log;errors=$errors;started=[DateTimeOffset]::Now.ToString('o');mode='drain-existing';workers=$workers;maxStorageMiB=$maxStorageMiB;minFreeMiB=$minFreeMiB}
    [IO.File]::WriteAllText($drainState,($record|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
    $script:state.crawlerPid=$process.Id;$script:state.workers=$workers;Write-State
    Log "Crawler started: PID $($process.Id), $workers workers, maximum $maxStorageMiB MiB, reserve $minFreeMiB MiB."
    return $process
}
function Restore-Services {
    foreach($name in @($script:state.stoppedServices)){
        if($name -notin $serviceNames){continue}
        try{
            $service=Get-Service -Name $name
            if($service.Status -ne 'Running'){Start-Service -Name $name;Log "Service restored: $name"}
        }catch{Log "Service restore failed for ${name}: $($_.Exception.Message)"}
    }
}
function Finish-Night {
    [IO.File]::WriteAllText($nightStop,'end of overnight boost')
    try{
        $current=Get-Crawler
        if($current.Count -gt 0 -and $script:state.workers -ne 12){Stop-Crawler}
        $budget=Get-Budget
        if((Get-Crawler).Count -eq 0 -and $script:state.wasCrawlerRunning -and $budget.freeMiB -ge $minFreeMiB -and $budget.storageMiB -lt $maxStorageMiB -and $budget.availableMemoryMiB -ge 1024){
            $null=Start-Crawler 12
        }
    }catch{Log "Daytime crawler transition: $($_.Exception.Message)"}
    Restore-Services
    $script:state.status='restored';$script:state.finishedAt=[DateTimeOffset]::Now.ToString('o');Write-State
    Log 'Overnight boost finished; optional services restored, no application uninstalled.'
}

if($Action -eq 'Status'){
    $state=Read-State
    if(!$state){'No overnight session configured.';exit}
    $state|ConvertTo-Json -Depth 8
    Get-Budget|ConvertTo-Json
    exit
}
if($Action -eq 'Restore'){
    $script:state=Read-State
    if(!$script:state -or $script:state.status -eq 'restored'){exit}
    [IO.File]::WriteAllText($nightStop,'scheduled restoration')
    $controller=Get-CimInstance Win32_Process -Filter "ProcessId=$($script:state.controllerPid)"
    if($controller -and $controller.CommandLine.Contains($scriptPath) -and $controller.ProcessId -ne $PID){
        $limit=[DateTime]::UtcNow.AddSeconds(180)
        while([DateTime]::UtcNow -lt $limit){
            Start-Sleep -Seconds 3
            $current=Read-State
            if($current.status -eq 'restored'){exit}
            if(!(Get-Process -Id $controller.ProcessId -ErrorAction SilentlyContinue)){break}
        }
        $script:state=Read-State
    }
    Finish-Night
    exit
}
$end=if($Until){[DateTimeOffset]::Parse($Until)}else{
    $value=[DateTimeOffset]::Now.Date.AddHours(13)
    if($value -le [DateTime]::Now){$value=$value.AddDays(1)}
    [DateTimeOffset]$value
}
if($end -le [DateTimeOffset]::Now -or $end -gt [DateTimeOffset]::Now.AddHours(14)){throw 'The overnight window must end within the next fourteen hours.'}
if($Action -eq 'Plan'){
    @{until=$end.ToString('o');maxWorkers=32;dayWorkers=12;maxStorageMiB=$maxStorageMiB;minFreeMiB=$minFreeMiB;budget=(Get-Budget);services=$serviceNames;cleanup='Classify pending Wikimedia edit/history/red links as skipped; preserve all rows and pages.'}|ConvertTo-Json -Depth 5
    exit
}
if($Action -eq 'Start'){
    $old=Read-State
    if($old -and $old.status -notin @('restored','failed')){
        $active=Get-CimInstance Win32_Process -Filter "ProcessId=$($old.controllerPid)"
        if($active -and $active.CommandLine.Contains($scriptPath)){throw 'An overnight controller already exists.'}
        throw 'Restore the previous overnight state before starting again.'
    }
    $arguments=@('-NoProfile','-NonInteractive','-File',"`"$scriptPath`"",'-Action','Run','-Until',$end.ToString('o'))
    $controller=Start-Process -FilePath $shell -ArgumentList $arguments -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput (Join-Path $dir 'night-controller.log') -RedirectStandardError (Join-Path $dir 'night-controller.err.log') -PassThru
    @{controllerPid=$controller.Id;until=$end.ToString('o')}|ConvertTo-Json
    exit
}

$lock=$null;$cleanup=$null;$awake=$false
try{
    $lock=[IO.File]::Open((Join-Path $dir 'night-controller.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    if(!(Test-Path -LiteralPath (Join-Path $dir 'vertex.db')) -or !(Test-Path -LiteralPath $binary)){throw 'Existing crawler and database required.'}
    $script:state=@{status='preparing';controllerPid=$PID;startedAt=[DateTimeOffset]::Now.ToString('o');until=$end.ToString('o');stoppedServices=@();closedApps=@();wasCrawlerRunning=((Get-Crawler).Count -gt 0);workers=12;maxWorkers=32;maxStorageMiB=$maxStorageMiB;minFreeMiB=$minFreeMiB;hardStopReason=$null}
    Write-State
    $taskName='Vertex-Night-Restore-'+$end.ToString('yyyyMMdd-HHmm')
    $taskAction=New-ScheduledTaskAction -Execute $shell -Argument "-NoProfile -NonInteractive -WindowStyle Hidden -File `"$scriptPath`" -Action Restore"
    $trigger=New-ScheduledTaskTrigger -Once -At $end.LocalDateTime
    $settings=New-ScheduledTaskSettingsSet -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Minutes 10)
    $principal=New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Highest
    $null=Register-ScheduledTask -TaskName $taskName -Action $taskAction -Trigger $trigger -Settings $settings -Principal $principal -Description 'Restore only services temporarily stopped for Vertex overnight crawling; no recurring schedule.' -Force
    $script:state.restoreTask=$taskName;Write-State
    Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class VertexNightPower { [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags); }'
    if([VertexNightPower]::SetThreadExecutionState([uint32]2147483649) -eq 0){throw 'Unable to keep the computer awake safely.'}
    $awake=$true
    foreach($name in $serviceNames){
        try{
            $service=Get-Service -Name $name -ErrorAction SilentlyContinue
            if(!$service -or $service.Status -ne 'Running' -or !$service.CanStop){continue}
            if(@($service.DependentServices|Where-Object Status -eq 'Running').Count -gt 0){continue}
            $script:state.stoppedServices+=@($name);Write-State
            Stop-Service -Name $name -NoWait
            $service.WaitForStatus('Stopped',[TimeSpan]::FromSeconds(20))
            Log "Service temporarily stopped: $name"
        }catch{Log "Optional service unchanged or pending ${name}: $($_.Exception.Message)"}
    }
    $apps=Get-Process -Name Spotify,Discord,steamunlocked-launcher -ErrorAction SilentlyContinue
    foreach($app in $apps){
        try{
            $path=$app.Path
            $allowed=($app.ProcessName -eq 'Spotify' -and $path -like '*\AppData\Roaming\Spotify\Spotify.exe') -or ($app.ProcessName -eq 'Discord' -and $path -like '*\AppData\Local\Discord\app-*\Discord.exe') -or ($app.ProcessName -eq 'steamunlocked-launcher' -and $path -like '*\SteamUnlocked Launcher\steamunlocked-launcher.exe')
            if(!$allowed){continue}
            if($app.MainWindowHandle -ne 0){$null=$app.CloseMainWindow()}
            $script:state.closedApps+=@(@{name=$app.ProcessName;pid=$app.Id;path=$path})
        }catch{Log "Optional app inspect failed: $($_.Exception.Message)"}
    }
    Write-State
    Start-Sleep -Seconds 4
    foreach($app in $script:state.closedApps){
        $current=Get-Process -Id $app.pid -ErrorAction SilentlyContinue
        if($current -and $current.Path -eq $app.path){
            try{Stop-Process -Id $current.Id -Force -ErrorAction Stop;Log "Optional app closed: $($app.name)"}
            catch{Log "Optional app left running: $($app.name): $($_.Exception.Message)"}
        }
    }
    Stop-Crawler
    if(Test-Path -LiteralPath $nightStop){Remove-Item -LiteralPath $nightStop}
    $budget=Get-Budget
    $workers=if($budget.availableMemoryMiB -ge 3072){32}elseif($budget.availableMemoryMiB -ge 2048){24}else{16}
    $crawler=Start-Crawler $workers
    $node=(Get-Command node).Source
    $cleanScript=Join-Path $PSScriptRoot 'clean-queue.mjs'
    $cleanup=Start-Process -FilePath $node -ArgumentList @("`"$cleanScript`"","`"$(Join-Path $dir 'vertex.db')`"",$end.ToString('o'),"`"$nightStop`"") -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput (Join-Path $dir 'night-cleanup.log') -RedirectStandardError (Join-Path $dir 'night-cleanup.err.log') -PassThru
    $script:state.cleanupPid=$cleanup.Id;$script:state.status='running';Write-State
    $lowMemory=0;$healthy=0;$restarts=0;$tick=0
    while([DateTimeOffset]::Now -lt $end -and !(Test-Path -LiteralPath $nightStop)){
        $budget=Get-Budget
        $current=Get-Crawler
        $script:state.budget=$budget;$script:state.lastCheck=[DateTimeOffset]::Now.ToString('o')
        if($budget.freeMiB -lt $minFreeMiB -or $budget.storageMiB -ge $maxStorageMiB){
            $script:state.hardStopReason='Disk reserve or database quota reached.';Write-State
            Stop-Crawler;[IO.File]::WriteAllText($nightStop,'disk safety limit');break
        }
        if($budget.availableMemoryMiB -lt 1536){$lowMemory++}else{$lowMemory=0}
        if($budget.availableMemoryMiB -ge 3072){$healthy++}else{$healthy=0}
        $workingMiB=0
        if($current.Count -gt 0){$workingMiB=(Get-Process -Id $current[0].ProcessId).WorkingSet64/1MB}
        if($lowMemory -ge 2 -or $workingMiB -gt 2304){
            Stop-Crawler
            $workers=[math]::Max(8,$workers-8)
            if((Get-Budget).availableMemoryMiB -ge 1024){$crawler=Start-Crawler $workers}else{Log 'Memory pressure: crawl paused until memory recovers.'}
            $lowMemory=0;$healthy=0
        }elseif($current.Count -eq 0){
            $record=Get-Content -LiteralPath $drainState -Raw|ConvertFrom-Json
            $tail=if(Test-Path -LiteralPath $record.log){Get-Content -LiteralPath $record.log -Tail 4}else{@()}
            $errors=if(Test-Path -LiteralPath $record.errors){Get-Content -LiteralPath $record.errors -Tail 4}else{@()}
            if(($tail -join ' ') -match 'Traitement de la file termine' -or ($errors -join ' ') -match 'Quota de stockage|Reserve d.espace'){
                $script:state.hardStopReason='Queue completed or crawler safety stop.';Write-State;break
            }
            if($budget.availableMemoryMiB -ge 2048 -and $restarts -lt 3){
                $restarts++;Start-Sleep -Seconds 30
                if([DateTimeOffset]::Now -lt $end -and !(Test-Path -LiteralPath $nightStop)){$crawler=Start-Crawler $workers}
            }elseif($restarts -ge 3){$script:state.hardStopReason='Three crawler restart attempts exhausted.';Write-State;break}
        }elseif($healthy -ge 80 -and $workers -lt 32){
            Stop-Crawler;$workers=[math]::Min(32,$workers+8);$crawler=Start-Crawler $workers;$healthy=0
        }
        if($tick%4 -eq 0){Log "Health: $workers workers, free RAM $($budget.availableMemoryMiB) MiB, database $($budget.storageMiB) MiB, free disk $($budget.freeMiB) MiB."}
        Write-State;$tick++
        Start-Sleep -Seconds 15
    }
}catch{
    Log "Controller error: $($_.Exception.Message)"
    if($script:state){$script:state.error=$_.Exception.Message;Write-State}
}finally{
    if($script:state){
        [IO.File]::WriteAllText($nightStop,'supervisor finished')
        if($cleanup){$null=$cleanup.WaitForExit(60000)}
        Finish-Night
    }
    if($awake){$null=[VertexNightPower]::SetThreadExecutionState([uint32]2147483648)}
    if($lock){$lock.Dispose()}
}
