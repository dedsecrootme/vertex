'use strict';
const state = document.querySelector('#releaseState');
async function loadRelease() {
  try {
    const response = await fetch('/api/release', {cache:'no-store',signal:AbortSignal.timeout(10000)});
    if (!response.ok) throw new Error('service');
    const release = await response.json();
    if (!release.available) {state.textContent='Nouvelle version en préparation.';return;}
    if (!/^\d+\.\d+\.\d+$/.test(release.version) || release.windows?.url!=='/downloads/windows' || !/^[a-f0-9]{64}$/.test(release.windows.sha256) || !Number.isSafeInteger(release.windows.size) || release.windows.size<=0) throw new Error('release');
    state.textContent=`Version ${release.version} · ${(release.windows.size/1048576).toFixed(1)} Mo · Windows x64`;
    state.classList.add('available');
    const button=document.querySelector('#downloadButton');
    button.href='/downloads/windows';button.hidden=false;
    document.querySelector('#downloadLabel').textContent=`Télécharger Vertex ${release.version}`;
    const checksum=document.querySelector('#checksum');checksum.textContent='SHA-256 : '+release.windows.sha256;checksum.hidden=false;
  } catch {state.textContent='Vérification indisponible. Réessayez dans quelques instants.';}
}
loadRelease();

const views={
  search:{src:'/search-view.png',name:'Recherche',alt:'Capture réelle de Vertex : résultats publics Rust sélectionnés',caption:'Des résultats publics sélectionnés dans l’index local de Vertex.'},
  reader:{src:'/reader-view.png',name:'Lecture',alt:'Capture réelle de Vertex : lecture des annonces publiques du langage Rust',caption:'Une page publique conservée dans l’index, ouverte dans la vue de lecture.'},
  crawler:{src:'/crawler-view.png',name:'Exploration',alt:'Capture réelle de Vertex : douze workers et progression du crawler le 19 octobre 2026',caption:'Une capture de la supervision locale. Les compteurs présentés ne sont pas en direct.'},
};
const tabs=[...document.querySelectorAll('.product-tab')];
const preview=document.querySelector('#productPreview');
const dialog=document.querySelector('#previewDialog');
let currentView='search', pendingView='search', viewVersion=0;
function selectTab(name) {
  tabs.forEach(tab=>{const selected=tab.dataset.view===name;tab.setAttribute('aria-selected',String(selected));tab.tabIndex=selected?0:-1;});
  document.querySelector('#product-view').setAttribute('aria-labelledby','tab-'+name);
}
async function selectView(name) {
  const view=views[name];if(!view)return;
  const version=++viewVersion;pendingView=name;
  selectTab(name);
  document.querySelector('#product-view').setAttribute('aria-busy','true');
  document.querySelector('#expandPreview').disabled=true;
  document.querySelector('#previewError').hidden=true;
  document.querySelector('#galleryState').textContent='Chargement de la vue '+view.name;
  try {
    const image=new Image();image.src=view.src;await image.decode();
    if(version!==viewVersion)return;
    preview.src=view.src;preview.alt=view.alt;currentView=name;
    animateSurface(preview);
    document.querySelector('#viewName').textContent=view.name;
    document.querySelector('#viewCaption').textContent=view.caption;
    document.querySelector('#galleryState').textContent='Vue '+view.name+' chargée';
  } catch {
    if(version!==viewVersion)return;
    selectTab(currentView);
    document.querySelector('#previewError').hidden=false;
    document.querySelector('#galleryState').textContent='Chargement impossible';
  } finally {
    if(version===viewVersion){document.querySelector('#product-view').setAttribute('aria-busy','false');document.querySelector('#expandPreview').disabled=false;}
  }
}
document.querySelector('#retryPreview').addEventListener('click',()=>selectView(pendingView));
preview.addEventListener('error',()=>{document.querySelector('#previewError').hidden=false;});
if(preview.complete&&!preview.naturalWidth)document.querySelector('#previewError').hidden=false;
tabs.forEach(tab=>tab.addEventListener('click',()=>selectView(tab.dataset.view)));
document.querySelector('.product-tabs').addEventListener('keydown',event=>{
  if(!['ArrowLeft','ArrowRight','Home','End'].includes(event.key))return;
  const current=tabs.indexOf(document.activeElement);
  const next=event.key==='Home'?0:event.key==='End'?tabs.length-1:(current+(event.key==='ArrowRight'?1:-1)+tabs.length)%tabs.length;
  event.preventDefault();tabs[next].focus();selectView(tabs[next].dataset.view);
});
document.querySelector('#expandPreview').addEventListener('click',()=>{
  const image=document.querySelector('#largePreview');image.src=preview.src;image.alt=preview.alt;
  dialog.showModal();document.body.classList.add('dialog-open');
});
document.querySelector('#closePreview').addEventListener('click',()=>dialog.close());
dialog.addEventListener('click',event=>{if(event.target===dialog){const rect=dialog.getBoundingClientRect();if(event.clientX<rect.left||event.clientX>rect.right||event.clientY<rect.top||event.clientY>rect.bottom)dialog.close();}});
dialog.addEventListener('close',()=>{document.body.classList.remove('dialog-open');document.querySelector('#expandPreview').focus();});

const menu=document.querySelector('#menuButton');
const nav=document.querySelector('#navLinks');
function closeMenu(){menu.setAttribute('aria-expanded','false');menu.setAttribute('aria-label','Ouvrir le menu');menu.title='Ouvrir le menu';nav.classList.remove('is-open');menu.querySelector('use').setAttribute('href','/icons.svg#menu');}
menu.addEventListener('click',()=>{if(menu.getAttribute('aria-expanded')==='true'){closeMenu();return;}menu.setAttribute('aria-expanded','true');menu.setAttribute('aria-label','Fermer le menu');menu.title='Fermer le menu';nav.classList.add('is-open');menu.querySelector('use').setAttribute('href','/icons.svg#x');});
nav.querySelectorAll('a').forEach(link=>link.addEventListener('click',closeMenu));
document.addEventListener('keydown',event=>{if(event.key==='Escape'&&menu.getAttribute('aria-expanded')==='true'){closeMenu();menu.focus();}});
document.addEventListener('click',event=>{if(!event.target.closest('.navigation'))closeMenu();});

const motion=matchMedia('(prefers-reduced-motion: reduce)');
let animate;
const effects=new Set();
// Keep every navigation and gallery action functional if the optional library fails.
import('/vendor/motion-mini.js').then(module=>{animate=module.animate;}).catch(()=>{});
function animateSurface(element) {
  if(motion.matches||!animate)return;
  for(const effect of effects)effect.cancel();
  effects.clear();
  const effect=animate(element,{opacity:[.4,1],transform:['translateX(12px)','translateX(0px)']},{duration:.45,ease:[.22,1,.36,1]});
  effects.add(effect);
  Promise.resolve(effect).then(()=>effects.delete(effect),()=>effects.delete(effect));
}
motion.addEventListener('change',()=>{if(motion.matches){for(const effect of effects)effect.cancel();effects.clear();}});
document.querySelectorAll('[data-open-view]').forEach(link=>link.addEventListener('click',()=>selectView(link.dataset.openView)));
const navigation=document.querySelector('.navigation');
let scrollFrame=0;
function updateNavigation(){scrollFrame=0;navigation.classList.toggle('is-scrolled',scrollY>24);}
addEventListener('scroll',()=>{if(!scrollFrame)scrollFrame=requestAnimationFrame(updateNavigation);},{passive:true});
updateNavigation();
if('IntersectionObserver' in window){
  const sections=new IntersectionObserver(entries=>{
    const visible=entries.find(entry=>entry.isIntersecting);
    if(!visible)return;
    nav.querySelectorAll('a').forEach(link=>{if(link.hash==='#'+visible.target.id)link.setAttribute('aria-current','true');else link.removeAttribute('aria-current');});
  },{rootMargin:'-20% 0px -55% 0px'});
  document.querySelectorAll('main>section[id]').forEach(section=>sections.observe(section));
}
if('IntersectionObserver' in window&&!motion.matches){
  document.body.classList.add('motion-enabled');
  const reveal=new IntersectionObserver(entries=>entries.forEach(entry=>{if(entry.isIntersecting){entry.target.classList.add('revealed');reveal.unobserve(entry.target);}}),{threshold:.12});
  document.querySelectorAll('[data-reveal]').forEach(el=>reveal.observe(el));
  motion.addEventListener('change',event=>{if(event.matches){document.body.classList.remove('motion-enabled');reveal.disconnect();}});
}
