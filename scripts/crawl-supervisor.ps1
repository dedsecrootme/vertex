param(
    [ValidateSet('Start','Run','Stop','Status')][string]$Action='Status',
    [ValidateRange(1,64)][int]$Workers=64,
    [ValidateSet('debug','release')][string]$Profile='release'
)
$ErrorActionPreference='Stop'
$project=Get-Item -LiteralPath (Split-Path -Parent $PSScriptRoot)
$resolved=$project.ResolveLinkTarget($true)
$root=if($resolved){$resolved.FullName}else{$project.FullName}
$dir=Join-Path $root 'database'
$launcher=Join-Path $root 'scripts/crawl-index.ps1'
$scriptPath=Join-Path $root 'scripts/crawl-supervisor.ps1'
$statePath=Join-Path $dir 'supervisor-state.json'
$stop=Join-Path $dir 'supervisor.stop'
$cleanupStop=Join-Path $dir 'cleanup.stop'
$log=Join-Path $dir 'supervisor.log'
$maxStorageMiB=81920
$minFreeMiB=51200
function Log([string]$message){[IO.File]::AppendAllText($log,"$([DateTimeOffset]::Now.ToString('o')) $message`r`n")}
function Save {
    $temp="$statePath.$PID.tmp"
    [IO.File]::WriteAllText($temp,($script:state|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temp -Destination $statePath -Force
}
function Budget {
    $os=Get-CimInstance Win32_OperatingSystem
    $size=0L
    foreach($name in @('vertex.db','vertex.db-wal')){if(Test-Path -LiteralPath (Join-Path $dir $name)){$size+=(Get-Item -LiteralPath (Join-Path $dir $name)).Length}}
    return @{memoryMiB=[math]::Floor($os.FreePhysicalMemory/1024);freeMiB=[math]::Floor([IO.DriveInfo]::new([IO.Path]::GetPathRoot($dir)).AvailableFreeSpace/1MB);storageMiB=[math]::Ceiling($size/1MB)}
}
function Get-Crawler {
    return @(Get-CimInstance Win32_Process -Filter "Name='crawler-rs.exe'"|Where-Object {$_.ExecutablePath -like "$root\target\*" -and $_.CommandLine.Contains($dir)})
}
function Stop-Crawler {
    $null=& $launcher -Action Stop
    $until=[DateTime]::UtcNow.AddSeconds(150)
    while((Get-Crawler).Count -gt 0){if([DateTime]::UtcNow -gt $until){throw 'Graceful stop timed out; crawler was not force-killed.'};Start-Sleep -Seconds 1}
}
function Launch([int]$count){
    $null=& $launcher -Action Start -Profile $Profile -Workers $count -MaxStorageMiB $maxStorageMiB -MinFreeMiB $minFreeMiB
    $record=Get-Content -LiteralPath (Join-Path $dir 'drain-process.json') -Raw|ConvertFrom-Json
    $script:state.workers=$count;$script:state.crawlerPid=$record.pid;Save
    Log "Started $Profile crawler with $count workers (PID $($record.pid))."
}
if($Action -eq 'Status'){
    if(Test-Path -LiteralPath $statePath){Get-Content -LiteralPath $statePath -Raw}
    & $launcher -Action Status
    exit
}
if($Action -eq 'Stop'){
    [IO.File]::WriteAllText($stop,'operator requested stop')
    [IO.File]::WriteAllText($cleanupStop,'operator requested stop')
    & $launcher -Action Stop
    exit
}
if($Action -eq 'Start'){
    if(Test-Path -LiteralPath $statePath){
        $old=Get-Content -LiteralPath $statePath -Raw|ConvertFrom-Json
        $existing=Get-CimInstance Win32_Process -Filter "ProcessId=$($old.pid)"
        if($existing -and $existing.CommandLine.Contains($scriptPath)){throw 'A supervisor is already running.'}
    }
    if((Get-Crawler).Count -gt 0){throw 'Stop the existing crawler before starting supervision.'}
    foreach($file in @($stop,$cleanupStop)){if(Test-Path -LiteralPath $file){Remove-Item -LiteralPath $file}}
    $shell=(Get-Command pwsh).Source
    $process=Start-Process -FilePath $shell -ArgumentList @('-NoProfile','-NonInteractive','-File',"`"$scriptPath`"",'-Action','Run','-Workers',"$Workers",'-Profile',$Profile) -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput (Join-Path $dir 'supervisor-controller.log') -RedirectStandardError (Join-Path $dir 'supervisor-controller.err.log') -PassThru
    @{pid=$process.Id;database=$dir;profile=$Profile;maxWorkers=$Workers}|ConvertTo-Json
    exit
}
$lock=$null;$cleanup=$null;$awake=$false
try{
    $lock=[IO.File]::Open((Join-Path $dir 'supervisor.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    $previousState=if(Test-Path -LiteralPath $statePath){Get-Content -LiteralPath $statePath -Raw|ConvertFrom-Json}else{$null}
    $script:state=@{pid=$PID;status='starting';database=$dir;profile=$Profile;maxWorkers=$Workers;maxStorageMiB=$maxStorageMiB;minFreeMiB=$minFreeMiB;startedAt=[DateTimeOffset]::Now.ToString('o')}
    Save
    Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class VertexCrawlPower { [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags); }'
    $awake=([VertexCrawlPower]::SetThreadExecutionState([uint32]2147483649) -ne 0)
    $budget=Budget
    $currentWorkers=if($budget.memoryMiB -ge 4096){$Workers}elseif($budget.memoryMiB -ge 2048){[math]::Min($Workers,32)}elseif($budget.memoryMiB -ge 1536){[math]::Min($Workers,16)}elseif($budget.memoryMiB -ge 1024){[math]::Min($Workers,8)}else{[math]::Min($Workers,4)}
    if($budget.memoryMiB -lt 4096 -and $previousState.profile -eq $Profile -and $previousState.workers -ge 1 -and $previousState.workers -le 64){$currentWorkers=[math]::Min($currentWorkers,[int]$previousState.workers)}
    Launch $currentWorkers
    $node=(Get-Command node).Source
    $deadline=[DateTimeOffset]::Now.AddHours(24).ToString('o')
    $cleanScript=Join-Path $root 'scripts/clean-queue.mjs'
    $cleanup=Start-Process -FilePath $node -ArgumentList @("`"$cleanScript`"","`"$(Join-Path $dir 'vertex.db')`"",$deadline,"`"$cleanupStop`"") -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput (Join-Path $dir 'cleanup-current.log') -RedirectStandardError (Join-Path $dir 'cleanup-current.err.log') -PassThru
    $script:state.cleanupPid=$cleanup.Id;$script:state.status='running';Save
    $pressure=0;$healthy=0;$restarts=0;$ticks=0;$stableSince=[DateTime]::UtcNow
    while(!(Test-Path -LiteralPath $stop)){
        $budget=Budget
        $script:state.budget=$budget;$script:state.updatedAt=[DateTimeOffset]::Now.ToString('o');Save
        if($budget.freeMiB -lt $minFreeMiB -or $budget.storageMiB -ge $maxStorageMiB){$script:state.reason='storage-limit';break}
        $memoryFloor=if($currentWorkers -le 4){768}else{1280}
        if($budget.memoryMiB -lt $memoryFloor){$pressure++}else{$pressure=0}
        if($budget.memoryMiB -ge 4096){$healthy++}else{$healthy=0}
        $active=Get-Crawler
        $native=if($active.Count){Get-Process -Id $active[0].ProcessId -ErrorAction SilentlyContinue}else{$null}
        $resident=if($native){$native.WorkingSet64/1MB}else{0}
        if($pressure -ge 3 -or $resident -gt 2048){
            Stop-Crawler
            $currentWorkers=[math]::Max(1,[math]::Floor($currentWorkers/2))
            if((Budget).memoryMiB -lt 1024){
                $script:state.status='paused-memory';Save;Log 'Crawl paused for memory recovery; pending URLs retained.'
                while(!(Test-Path -LiteralPath $stop)){
                    $recovery=Budget
                    $script:state.budget=$recovery;$script:state.updatedAt=[DateTimeOffset]::Now.ToString('o');Save
                    if($recovery.freeMiB -lt $minFreeMiB -or $recovery.storageMiB -ge $maxStorageMiB){$script:state.reason='storage-limit';break}
                    $recoveryFloor=if($currentWorkers -le 4){1024}else{1536}
                    if($recovery.memoryMiB -ge $recoveryFloor){break}
                    Start-Sleep -Seconds 15
                }
                if((Test-Path -LiteralPath $stop) -or $script:state.reason -eq 'storage-limit'){break}
            }
            $script:state.status='running'
            Launch $currentWorkers;$pressure=0;$healthy=0
        }elseif(!$active.Count){
            $record=Get-Content -LiteralPath (Join-Path $dir 'drain-process.json') -Raw|ConvertFrom-Json
            $last=Get-Content -LiteralPath $record.log -Tail 4
            if(($last -join ' ') -match 'Traitement de la file termine'){$script:state.reason='queue-completed';break}
            if($restarts -ge 3){$script:state.reason='restart-limit';break}
            $restarts++;$stableSince=[DateTime]::UtcNow;Log "Unexpected crawler exit; restart $restarts of 3."
            Start-Sleep -Seconds 15
            if(!(Test-Path -LiteralPath $stop)){Launch $currentWorkers}
        }elseif($healthy -ge 80 -and $currentWorkers -lt $Workers){
            Stop-Crawler;$currentWorkers=[math]::Min($Workers,$currentWorkers+8);Launch $currentWorkers;$healthy=0
        }
        if($active.Count -gt 0 -and ([DateTime]::UtcNow-$stableSince).TotalMinutes -ge 10){$restarts=0}
        if($ticks%4 -eq 0){Log "Health: $currentWorkers workers; available RAM $($budget.memoryMiB) MiB; free disk $($budget.freeMiB) MiB; index $($budget.storageMiB) MiB."}
        $ticks++;Start-Sleep -Seconds 15
    }
}catch{
    if($script:state){$script:state.reason=$_.Exception.Message;Save}
    Log "Supervisor error: $($_.Exception.Message)"
}finally{
    if($lock){[IO.File]::WriteAllText($cleanupStop,'supervisor stopped')}
    if($script:state){
        try{Stop-Crawler}catch{Log $_.Exception.Message}
        $script:state.status='stopped';$script:state.finishedAt=[DateTimeOffset]::Now.ToString('o');Save
    }
    if($cleanup){$null=$cleanup.WaitForExit(60000)}
    if($awake){$null=[VertexCrawlPower]::SetThreadExecutionState([uint32]2147483648)}
    if($lock){$lock.Dispose()}
}
