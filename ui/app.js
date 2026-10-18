"use strict";
const MOTION_MS=400;
document.documentElement.style.setProperty("--motion-duration",MOTION_MS+"ms");
const $ = (id) => document.getElementById(id);
const invoke = window.__TAURI__?.core?.invoke;
for(const def of algorithmParameters){const input=document.createElement(def.type==="select"?"select":"input");input.id=def.id;if(def.type==="select"){for(const [value,label] of def.values){const option=document.createElement("option");option.value=value;option.textContent=label;input.append(option);}input.value=def.value;}else{input.type=def.type;if(def.type==="checkbox")input.checked=def.value;else{input.min=def.min;input.max=def.max;input.step="any";input.value=def.value;}}input.setAttribute("aria-label",def.name);document.querySelector(".control-values").append(input);const button=document.createElement("button");button.id=def.id+"-button";button.type="button";button.className="parameter-button";button.dataset.field=def.id;$(def.scope==="vcTrace"?"algorithm-scroll":"reduction-scroll").append(button);}

let images = [], selected = null, busy = false;
const fileOptions=new Map();
const library=new FileLibrary();let libraryRevision=0,librarySync=Promise.resolve();
function syncLibrary(){const snapshot=JSON.parse(JSON.stringify(library.nodes)),expanded=[...library.expanded],token=++libraryRevision;librarySync=librarySync.then(()=>invoke?.("set_library",{revision:token,library:snapshot,expanded})).catch(error=>status(String(error),true));}
const defaultOptions={colors:3,quantization:"mc",prescale:1,despeckle:2,smoothCorners:0,optimizePaths:5,transparency:"white",dither:"none",veclineIterations:4,veclineStrategy:"mean",tracing:"potrace",...Object.fromEntries(algorithmParameters.map(def=>[def.id,def.value]))};
function sourceKey(image){return FileLibrary.sourceKey(image);}
function loadFileOptions(image){
  const settings={...defaultOptions,...(fileOptions.get(sourceKey(image)) || image.options || {})};
  if(!["mc","vc","wl"].includes(settings.quantization))settings.quantization="wl";
  settings.veclineIterations=Math.max(0,Math.min(32,Math.round(Number(settings.veclineIterations)||0)));
  settings.transparency=settings.transparency==="white"?"white":"off";
  for(const id of fields){const input=$(id);if(input.type==="checkbox")input.checked=Boolean(settings[id]);else input.value=settings[id];}
  $("prescale").value=Math.max(.5,Math.min(8,Number($("prescale").value)));$("colors").max=String(Math.min(32,image.paletteColors||32));$("colors").value=Math.max(1,Math.min(Number($("colors").max),Number($("colors").value)));
  $("smoothCorners").value=Math.max(0,Math.min(1.3,Number($("smoothCorners").value)));
}
function rememberFileOptions(settings){const image=images.find(i=>i.id===selected);if(!image)return;fileOptions.set(sourceKey(image),{...settings});for(const entry of images)if(sourceKey(entry)===sourceKey(image))entry.options={...settings};}

let fullPreview=null, importEpoch=0; const previewRequests=new Map(), thumbCache=new Map(), thumbRequests=new Set(), thumbQueue=[], visibleThumbs=new Set(); let thumbActive=0;
let live = null, revision = 0, traceTimer = null, tracing = false, tracePending = false;
let split = 0.5, scale = null, previewId = null;
const pans = new Map(); let dragging = null, pointerHeight = .5;
let overlayGeometry = null, wipeFrame = null, wipePointer = null;
let vectorSource = null, vectorSourceKey = null, vectorLoad = 0, vectorRaster = null, vectorBitmap = null, rasterTimer = null,rasterRequest=0;
const RASTER_MARGIN = 96, RASTER_PIXELS = 4_194_304;
let originalRaster=null,originalTimer=null,originalRequest=0;
function formatBytes(bytes) { return bytes < 1024 ? `${bytes} B` : bytes < 1048576 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1048576).toFixed(2)} MB`; }
const fields = ["colors", "dither", "quantization", "prescale", "despeckle", "smoothCorners", "optimizePaths", "transparency", "veclineIterations","tracing",...algorithmParameters.map(def=>def.id)];
const messages=[];let unreadMessages=0,toastTimer=null;
const panelStates=new Map();
function syncFloatingButtons(){
  $("toggle-files").hidden=!$("file-panel").hidden;
  $("notifications").hidden=!messages.length||!$("notification-panel").hidden||!$("notification-toast").hidden;
}
function animatePanel(id,open){
  const panel=$(id),state=panelStates.get(id)||{open:false,timer:null};panelStates.set(id,state);
  if(state.open===open){syncFloatingButtons();return;}
  state.open=open;panel.dataset.open=String(open);clearTimeout(state.timer);if(state.end)panel.removeEventListener("animationend",state.end);
  panel.classList.remove("is-opening","is-closing");
  const reduced=window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  const finish=()=>{clearTimeout(state.timer);state.timer=null;if(state.end)panel.removeEventListener("animationend",state.end);state.end=null;panel.hidden=!state.open;panel.classList.remove("is-opening","is-closing");if(id==="notification-panel"&&panel.hidden&&!messages.length){$("notification-list").replaceChildren();$("notification-empty").hidden=false;}syncFloatingButtons();};
  if(open){panel.hidden=false;void panel.offsetWidth;panel.classList.add("is-opening");}
  else if(!panel.hidden){void panel.offsetWidth;panel.classList.add("is-closing");}
  syncFloatingButtons();if(reduced){finish();return;}state.end=event=>{if(event.target===panel)finish();};panel.addEventListener("animationend",state.end);state.timer=setTimeout(finish,MOTION_MS+20);
}
function messageRow(message){
  const row=document.createElement("div");row.className=`notification-message ${message.error?"error":"success"}`;
  const body=document.createElement("p");body.textContent=message.text;row.append(body);
  if(message.path){row.classList.add("saved-file");row.setAttribute("role","button");row.tabIndex=0;row.setAttribute("aria-label","打开已保存文件的位置");
    const reveal=event=>{event.stopPropagation();invoke("reveal_file",{path:message.path}).catch(error=>status(String(error),true));};
    row.addEventListener("click",reveal);row.addEventListener("keydown",event=>{if(["Enter"," "].includes(event.key)){event.preventDefault();reveal(event);}});
  }
  return row;
}
function paintMessages(){
  if(messages.length||$("notification-panel").hidden){$("notification-list").replaceChildren(...messages.map(messageRow));$("notification-empty").hidden=messages.length>0;}$("notification-dot").hidden=unreadMessages===0;
  $("notifications").classList.toggle("has-error",unreadMessages>0&&messages[0]?.error);syncFloatingButtons();
}
function closeToast(){clearTimeout(toastTimer);toastTimer=null;animatePanel("notification-toast",false);}
function showToast(message){
  clearTimeout(toastTimer);$("toast-content").replaceChildren(messageRow(message));animatePanel("notification-toast",true);
  toastTimer=setTimeout(()=>{toastTimer=null;animatePanel("notification-toast",false);},5000);
}
function status(message,error=false,path=null){
  if(!message)return;const latest=messages[0];
  if(latest?.text===message&&latest.error===error&&Date.now()-latest.created<1500)return;
  const entry={text:message,error,path,created:Date.now()};messages.unshift(entry);messages.length=Math.min(messages.length,100);
  const expanded=panelStates.get("notification-panel")?.open;if(!expanded)unreadMessages++;paintMessages();if(!expanded)showToast(entry);
}
function setNotifications(open){
  if(open){closeToast();unreadMessages=0;paintMessages();}animatePanel("notification-panel",open&&messages.length>0);
  $("notifications").setAttribute("aria-expanded",String(open&&messages.length>0));
}
$("notifications").addEventListener("click",()=>setNotifications(!panelStates.get("notification-panel")?.open));
$("notification-toast").addEventListener("click",()=>setNotifications(true));
$("clear-notifications").addEventListener("click",()=>{messages.length=0;unreadMessages=0;closeToast();setNotifications(false);paintMessages();});
function current() { const image=images.find(image=>image.id===selected); return image ? {...image,preview:fullPreview?.id===selected?fullPreview.url:(image.preview||""),...(live?.id===selected?live:{})} : null; }
function discardPreview() { live=null; clearVector(); }
function selectImage(id) { if(selected===id)return; selected=id; const image=images.find(i=>i.id===id);if(image)loadFileOptions(image); fullPreview=null; discardPreview(); render(); invalidate(); }
function resultParameters(image) {
  const o = image.resultOptions;
  if (!o) return "";
  const colors = `${o.colors} 色`;
  const quantization = {mc:"Median cut",vc:"Vecline Lloyd",wl:"Oklab Wu + Lloyd"}[o.quantization] || o.quantization;
  const algorithm=o.quantization==="wl"?[`迭代 ${o.lloydIterations??4}`]:o.quantization==="vc"?[`取色 ${{mean:"平均",dominant:"主导",median:"中位"}[o.veclineStrategy||"mean"]}`,...(!o.veclineStrategy||o.veclineStrategy==="mean"?[`精修 ${o.veclineIterations??4}`]:[])]:[];
  return [colors, quantization,...(o.quantization!=="vc"||o.colors>1?algorithm:[`阈值 ${{auto:"Otsu",fixed:o.monoCutoff,adaptive:"Bradley"}[o.monoThreshold||"auto"]}`]), ...(o.dither&&o.dither!=="none"?[`拟色 ${o.dither==="floydsteinberg"?"Floyd–Steinberg":"Riemersma"}`]:[]), `描摹精度 ${o.prescale}`, o.tracing==="vecline"?`Vecline · ${o.vcMode||"trace"} · ${o.vcPreset||"default"} · 简化 ${o.vcTolerance} · 拟合 ${o.vcFitError} · 合并 ${o.vcOptimizeError}`:`Potrace · 过滤 ${o.despeckle} · 平滑 ${o.smoothCorners} · 优化 ${o.optimizePaths}`].join(" · ");
}
function options() {
  const value = {};
  for (const id of fields) {
    const input = $(id);
    if (!input.checkValidity()) throw new Error(`请检查「${names[id]}」的输入。`);
    value[id] = input.type === "checkbox" ? input.checked : input.type === "number" ? Number(input.value) : input.value;
  }
  value.stacked = true; value.veclineStrategy="mean";
  return value;
}
function updateButtons() {
  $("library-add").disabled=busy||!invoke;$("library-clear").disabled=busy||!images.length||!invoke;
  $("save").disabled = busy || tracing || tracePending || live?.revision!==revision || !current()?.result || !invoke;
  for (const id of fields) {if($(id))$(id).disabled = busy || !invoke;}
  for(const button of document.querySelectorAll(".file-remove"))button.disabled=busy;

  $("workspace").classList.toggle("is-empty",!images.length);
  refreshControls();
}
let queueDirty=true;
function queueUnits(){library.normalize(images);return library.flatten(images);}
async function ensurePreview(id){
  if(!invoke||previewRequests.has(id))return;
  const epoch=importEpoch,request=invoke("preview_image",{id,thumbnail:false});previewRequests.set(id,request);
  try{const url=await request;if(epoch===importEpoch&&selected===id&&images.some(image=>image.id===id)){fullPreview={id,url};renderPreview();}}
  catch(error){if(epoch===importEpoch&&selected===id)status(String(error),true);}finally{previewRequests.delete(id);if(epoch!==importEpoch&&selected===id)ensurePreview(id);}
}
function requestThumbnail(id){if(thumbCache.has(id)||thumbRequests.has(id)||!invoke)return;thumbRequests.add(id);thumbQueue.push(id);pumpThumbnails();}
async function pumpThumbnails(){
  while(thumbActive<2&&thumbQueue.length){const id=thumbQueue.shift();if(!visibleThumbs.has(id)){thumbRequests.delete(id);continue;}thumbActive++;
    const epoch=importEpoch;
    invoke("preview_image",{id,thumbnail:true}).then(url=>{if(epoch!==importEpoch||!images.some(image=>image.id===id))return;thumbCache.set(id,url);while(thumbCache.size>96)thumbCache.delete(thumbCache.keys().next().value);if(visibleThumbs.has(id)){for(const image of document.querySelectorAll(`img[data-image-id="${id}"]`))image.src=url;}}).catch(error=>{if(epoch===importEpoch)status(`缩略图读取失败：${error}`,true);}).finally(()=>{thumbRequests.delete(id);thumbActive--;if(epoch!==importEpoch&&visibleThumbs.has(id))requestThumbnail(id);pumpThumbnails();});
  }
}
const thumbnailObserver=window.IntersectionObserver?new IntersectionObserver(entries=>{
  const changed=new Set();for(const entry of entries){if(!document.contains(entry.target))continue;entry.target.dataset.thumbVisible=String(entry.isIntersecting);changed.add(Number(entry.target.dataset.imageId));}
  for(const id of changed){const thumbnails=[...$("files").querySelectorAll('img[data-image-id="'+id+'"]')];if(thumbnails.some(img=>img.dataset.thumbVisible==="true")){visibleThumbs.add(id);if(thumbCache.has(id))for(const img of thumbnails)img.src=thumbCache.get(id);else requestThumbnail(id);}else{visibleThumbs.delete(id);for(const img of thumbnails)img.removeAttribute("src");}}
},{root:$("file-panel"),rootMargin:"120px"}):null;
const FILE_HEIGHT=156,LIBRARY_GAP=8,LIST_MARGIN=312;let libraryLayout={entries:[],frames:[],slots:0};let virtualRows=[],virtualHeight=0,listFrame=null,libraryColumns=3;const rowCache=new Map();
let libraryDrag=null,suppressLibraryClick=0;
function queueChanged(){queueDirty=true;}
async function removeImages(ids){if(busy)return;busy=true;updateButtons();try{for(const id of ids)await invoke("forget_image",{id});const deleted=new Set(ids);images=images.filter(image=>!deleted.has(image.id));for(const id of ids){thumbCache.delete(id);rowCache.delete(`image:${id}`);}for(const key of fileOptions.keys())if(!images.some(image=>sourceKey(image)===key))fileOptions.delete(key);library.normalize(images);syncLibrary();queueChanged();if(deleted.has(selected)){selected=images[0]?.id??null;if(images[0])loadFileOptions(images[0]);fullPreview=null;discardPreview();invalidate();}}catch(error){status(String(error),true);}finally{busy=false;render();}}
function libraryTile(item){
  let row=rowCache.get(item.key);if(!row){row=document.createElement("div");row.className="file library-tile";const button=document.createElement("button");button.className="file-select";const art=document.createElement("span");art.className="library-art";const label=document.createElement("span");label.className="file-name";const state=document.createElement("span");state.className="file-state";button.append(art,label,state);const remove=document.createElement("button");remove.className="file-remove";remove.textContent="×";row.append(button,remove);
    button.addEventListener("click",()=>{if(Date.now()<suppressLibraryClick)return;const item=row.libraryItem;if(item.type==="image")selectImage(item.image.id);else{animateLibraryChange(()=>{library.toggle(item.key);return true;});}});
    remove.addEventListener("click",()=>{const item=row.libraryItem;if(item.type==="group"){animateLibraryChange(()=>library.ungroup(item.key),null,"ungroup");}else removeImages(item.type==="source"?images.filter(image=>sourceKey(image)===sourceKey(item.image)).map(image=>image.id):[item.image.id]);});
    row.draggable=false;button.draggable=false;label.draggable=false;
    row.addEventListener("dragstart",event=>event.preventDefault());
    row.addEventListener("pointerdown",event=>{if(event.button!==0||busy||event.target.closest(".file-remove")||!row.libraryItem.nodeKey)return;event.preventDefault();libraryDrag={row,key:row.libraryItem.nodeKey,x:event.clientX,y:event.clientY,id:event.pointerId,active:false,target:null};});
    rowCache.set(item.key,row);
  }
  row.libraryItem=item;row.dataset.nodeKey=item.nodeKey||"";row.dataset.libraryKey=item.key;row.dataset.depth=String(item.depth);row.classList.toggle("active",item.type==="image"&&item.image.id===selected);row.classList.toggle("is-container",item.type!=="image");row.classList.toggle("is-group",item.type==="group");row.classList.toggle("is-pdf",item.type==="source"&&Boolean(item.image.pdfGroup));row.classList.toggle("is-open",Boolean(item.open));
  const button=row.firstElementChild,art=button.querySelector(".library-art");button.setAttribute("aria-label",item.label);button.title=item.type==="image"?item.label:`${item.label}：点击展开或收起，拖入文件分组`;button.querySelector(".file-name").textContent=item.label.replace(/\.[a-z0-9]+(?= · |$)/i,"");button.querySelector(".file-state").textContent=item.type==="image"?`${item.image.width} × ${item.image.height}`:"";
  if(item.type==="image"){art.classList.remove("thumbnail-stack");if(art.firstElementChild?.tagName!=="IMG"){const img=document.createElement("img");img.alt="";img.decoding="async";img.draggable=false;art.replaceChildren(img);}art.firstElementChild.dataset.imageId=String(item.image.id);row.dataset.imageId=String(item.image.id);button.removeAttribute("aria-expanded");}
  else {
    if(item.type==="group"){
      art.classList.add("thumbnail-stack");art.replaceChildren(...(item.previewImages||[]).map((image,index)=>{const img=document.createElement("img");img.alt="";img.decoding="async";img.draggable=false;img.dataset.imageId=String(image.id);img.style.setProperty("--stack-index",index);return img;}));
    }else{
      art.classList.remove("thumbnail-stack");art.innerHTML=item.image.pdfGroup?'<svg class="pdf-icon" viewBox="0 0 40 48" aria-hidden="true"><path d="M7 2h17l9 9v34H7zM24 2v10h9"/><text x="20" y="32" text-anchor="middle">PDF</text></svg>':'<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 3h10v14H7zM4 6v14h10M10 6h4M10 10h4"/></svg>';
    }
    button.setAttribute("aria-expanded",String(item.open));delete row.dataset.imageId;
  }
  row.lastElementChild.title=item.type==="group"?"拆开分组，保留组内文件":`移除 ${item.label}`;row.lastElementChild.setAttribute("aria-label",row.lastElementChild.title);row.lastElementChild.disabled=busy;return row;
}
function layoutLibrary(rows,columns){
  const entries=[],frames=[],open=[];let slot=0;
  const close=()=>{open.pop().endSlot=slot;};
  for(const item of rows){
    while(open.length&&item.depth<=open[open.length-1].depth)close();
    const framed=item.open&&(item.type==="group"||item.type==="source"&&item.image?.pdfGroup);
    entries.push({item,slot});
    if(framed){const frame={key:item.key,depth:item.depth,startSlot:slot,endSlot:0};frames.push(frame);open.push(frame);}
    slot++;
  }
  while(open.length)close();
  return {entries,frames,slots:slot};
}
function libraryFrameShape(frame,columns,width,columnWidth){
  const firstRow=Math.floor(frame.startSlot/columns),lastRow=Math.floor((frame.endSlot-1)/columns),inset=2+Math.min(frame.depth,3)*2,height=(lastRow-firstRow+1)*(FILE_HEIGHT+LIBRARY_GAP)-LIBRARY_GAP,segments=[];
  for(let row=firstRow;row<=lastRow;row++){
    const start=Math.max(frame.startSlot,row*columns),end=Math.min(frame.endSlot,(row+1)*columns),left=Math.max(inset,(start%columns)*(columnWidth+LIBRARY_GAP)-LIBRARY_GAP/2+inset),right=Math.min(width-inset,((end-1)%columns+1)*(columnWidth+LIBRARY_GAP)-LIBRARY_GAP/2-inset),top=(row-firstRow)*(FILE_HEIGHT+LIBRARY_GAP)+inset,bottom=(row-firstRow)*(FILE_HEIGHT+LIBRARY_GAP)+FILE_HEIGHT-inset,r=Math.min(12,(right-left)/2,(bottom-top)/2),openLeft=row>firstRow,openRight=row<lastRow;
    let path;
    if(openLeft&&openRight)path='M'+left+' '+top+'L'+right+' '+top+'M'+left+' '+bottom+'L'+right+' '+bottom;
    else if(openRight)path='M'+right+' '+top+'L'+(left+r)+' '+top+'Q'+left+' '+top+' '+left+' '+(top+r)+'L'+left+' '+(bottom-r)+'Q'+left+' '+bottom+' '+(left+r)+' '+bottom+'L'+right+' '+bottom;
    else if(openLeft)path='M'+left+' '+top+'L'+(right-r)+' '+top+'Q'+right+' '+top+' '+right+' '+(top+r)+'L'+right+' '+(bottom-r)+'Q'+right+' '+bottom+' '+(right-r)+' '+bottom+'L'+left+' '+bottom;
    else path='M'+(left+r)+' '+top+'L'+(right-r)+' '+top+'Q'+right+' '+top+' '+right+' '+(top+r)+'L'+right+' '+(bottom-r)+'Q'+right+' '+bottom+' '+(right-r)+' '+bottom+'L'+(left+r)+' '+bottom+'Q'+left+' '+bottom+' '+left+' '+(bottom-r)+'L'+left+' '+(top+r)+'Q'+left+' '+top+' '+(left+r)+' '+top+'Z';
    segments.push({row,start,end,left,right,top,bottom,openLeft,openRight,path});
  }
  return {top:firstRow*(FILE_HEIGHT+LIBRARY_GAP),height,path:segments.map(segment=>segment.path).join(''),segments};
}
function firstLayoutSlot(slot){let left=0,right=libraryLayout.entries.length;while(left<right){const mid=(left+right)>>>1;if(libraryLayout.entries[mid].slot<slot)left=mid+1;else right=mid;}return left;}
function renderVirtualQueue(){if($("file-panel").hidden)return;thumbnailObserver?.disconnect();visibleThumbs.clear();const root=$("files"),panel=$("file-panel"),width=root.clientWidth||Math.max(0,panel.clientWidth-28)||480;libraryColumns=Math.max(2,Math.floor((width+LIBRARY_GAP)/(132+LIBRARY_GAP)));const columnWidth=(width-(libraryColumns-1)*LIBRARY_GAP)/libraryColumns;libraryLayout=layoutLibrary(virtualRows,libraryColumns);virtualHeight=Math.ceil(libraryLayout.slots/libraryColumns)*(FILE_HEIGHT+LIBRARY_GAP);root.style.height=`${virtualHeight}px`;
  const top=Math.max(0,panel.scrollTop-root.offsetTop),height=panel.clientHeight||Math.min(window.innerHeight-80,600),low=firstLayoutSlot(Math.max(0,Math.floor((top-LIST_MARGIN)/(FILE_HEIGHT+LIBRARY_GAP))*libraryColumns)),high=firstLayoutSlot(Math.ceil((top+height+LIST_MARGIN)/(FILE_HEIGHT+LIBRARY_GAP))*libraryColumns),fragment=document.createDocumentFragment(),visibleKeys=new Set();
  for(const frame of libraryLayout.frames){const shape=libraryFrameShape(frame,libraryColumns,width,columnWidth);if(shape.top+shape.height<top-LIST_MARGIN||shape.top>top+height+LIST_MARGIN)continue;const ns="http://www.w3.org/2000/svg",rim=document.createElementNS(ns,"svg"),path=document.createElementNS(ns,"path");rim.classList.add("library-container-frame");rim.dataset.containerKey=frame.key;rim.setAttribute("aria-hidden","true");rim.setAttribute("viewBox",'0 0 '+width+' '+shape.height);path.setAttribute("d",shape.path);rim.append(path);Object.assign(rim.style,{top:shape.top+"px",left:"0px",width:width+"px",height:shape.height+"px"});fragment.append(rim);}
  for(let index=low;index<high;index++){const {item,slot}=libraryLayout.entries[index],row=libraryTile(item);visibleKeys.add(item.key);row.style.position="absolute";row.style.top=`${Math.floor(slot/libraryColumns)*(FILE_HEIGHT+LIBRARY_GAP)}px`;row.style.left=`${(slot%libraryColumns)*(columnWidth+LIBRARY_GAP)}px`;row.style.right="auto";row.style.width=`${columnWidth}px`;row.style.height=`${FILE_HEIGHT}px`;fragment.append(row);}root.replaceChildren(fragment);
  for(const row of rowCache.values())if(!root.contains(row))row.querySelectorAll("img").forEach(img=>img.removeAttribute("src"));for(const[key]of rowCache)if(rowCache.size>96&&!visibleKeys.has(key))rowCache.delete(key);
  for(const thumbnail of root.querySelectorAll("img")){const id=Number(thumbnail.dataset.imageId),image=images.find(image=>image.id===id);if(image?.preview)thumbnail.src=image.preview;else if(thumbnailObserver)thumbnailObserver.observe(thumbnail);else{visibleThumbs.add(id);if(thumbCache.has(id))thumbnail.src=thumbCache.get(id);else requestThumbnail(id);}}
}
function renderQueue(){if($("file-panel").hidden)return;$("queue-empty").hidden=images.length>0;if(queueDirty){queueDirty=false;library.normalize(images);virtualRows=library.flatten(images);renderVirtualQueue();}else for(const row of $("files").querySelectorAll(".file"))row.classList.toggle("active",row.libraryItem.type==="image"&&row.libraryItem.image.id===selected);}
const libraryAnimations=new Set();let libraryPanelAnimation=null;
function libraryPaths(){const paths=new Map();const walk=(nodes,parents)=>{for(const node of nodes){const key=library.key(node);paths.set(key,parents);if(node.kind==="group")walk(node.children,[...parents,key]);}};walk(library.nodes,[]);return paths;}
function animateLibraryChange(mutate,flight=null,motion="toggle"){
  const root=$("files"),panel=$("file-panel"),before=new Map(),oldPaths=libraryPaths(),panelHeight=panel.getBoundingClientRect().height;
  if(libraryPanelAnimation){libraryPanelAnimation.cancel();libraryPanelAnimation=null;}panel.style.height="";
  const rootRect=root.getBoundingClientRect();
  for(const row of root.children){if(!row.classList.contains("library-tile"))continue;const rect=row.getBoundingClientRect();before.set(row.dataset.libraryKey,{key:row.dataset.libraryKey,nodeKey:row.libraryItem?.nodeKey,left:rect.width?rect.left-rootRect.left:parseFloat(row.style.left)||0,top:rect.height?rect.top-rootRect.top:parseFloat(row.style.top)||0,width:rect.width||parseFloat(row.style.width)||132,height:rect.height||FILE_HEIGHT,clone:row.cloneNode(true)});}
  for(const animation of libraryAnimations)animation.cancel();libraryAnimations.clear();
  root.querySelectorAll(".library-transition").forEach(node=>node.remove());
  if(flight){const old=before.get(flight.row.dataset.libraryKey),rect=flight.ghost.getBoundingClientRect(),rootRect=root.getBoundingClientRect();if(old){old.left=rect.left-rootRect.left;old.top=rect.top-rootRect.top;old.clone=flight.ghost.cloneNode(true);old.clone.classList.remove("library-drag-preview");old.clone.style.position="absolute";old.clone.style.transform="none";old.clone.style.left=old.left+"px";old.clone.style.top=old.top+"px";}}
  if(!mutate())return false;library.normalize(images);syncLibrary();queueChanged();renderQueue();
  if(window.matchMedia?.("(prefers-reduced-motion: reduce)").matches)return true;
  const duration=MOTION_MS,panelDuration=MOTION_MS,targetHeight=panel.getBoundingClientRect().height;
  if(panel.animate&&panelHeight&&targetHeight&&Math.abs(targetHeight-panelHeight)>.5){
    panel.style.height=targetHeight+"px";const resizing=panel.animate([{height:panelHeight+"px"},{height:targetHeight+"px"}],{duration:panelDuration,easing:"cubic-bezier(.4,0,.2,1)",fill:"both"});libraryPanelAnimation=resizing;
    const finish=()=>{if(libraryPanelAnimation!==resizing)return;libraryPanelAnimation=null;panel.style.height="";resizing.cancel();renderVirtualQueue();};resizing.finished.then(finish,finish);
  }
  const afterPaths=libraryPaths(),after=new Map([...root.children].filter(row=>row.classList.contains("library-tile")).map(row=>[row.dataset.libraryKey,row]));
  const nearest=(parents,visible)=>[...(parents||[])].reverse().map(key=>visible.get(key)).find(Boolean);
  const animate=(row,frames,delay=0,remove=false)=>{
    if(!row.animate){if(remove)row.remove();return;}
    const animation=row.animate(frames,{duration,delay,easing:"cubic-bezier(.4,0,.2,1)",fill:"both"});libraryAnimations.add(animation);
    const finish=()=>{libraryAnimations.delete(animation);if(remove)row.remove();else animation.cancel();};animation.finished.then(finish,finish);
  };
  let arrival=0;
  for(const [key,row]of after){const old=before.get(key);let origin=old;
    if(!origin)origin=nearest(oldPaths.get(row.libraryItem?.nodeKey),before)||before.get(row.libraryItem?.nodeKey);
    if(!origin&&row.libraryItem?.type==="group")origin=[...before.values()].find(item=>afterPaths.get(item.nodeKey)?.includes(key));
    const left=parseFloat(row.style.left)||0,top=parseFloat(row.style.top)||0;
    if(origin){const x=origin.left-left,y=origin.top-top;
      if(old){if(x||y)animate(row,[{transform:'translate('+x+'px,'+y+'px)'},{transform:"none"}]);}
      else {const bend=arrival++%2?-12:12;animate(row,[{transform:'translate('+x+'px,'+y+'px) scale(.36) rotate('+bend+'deg)'},{offset:.65,transform:'translate('+x*.15+'px,'+(y*.15-14)+'px) scale(.94) rotate('+bend*.1+'deg)'},{transform:"none"}],0);}
    }else animate(row,[{transform:"scale(.86)"},{transform:"scale(1)"}]);
  }
  for(const [key,old]of before){if(after.has(key))continue;
    const target=nearest(afterPaths.get(old.nodeKey),after)||[...after.values()].find(row=>oldPaths.get(row.libraryItem?.nodeKey)?.includes(key));
    const ghost=old.clone;ghost.classList.add("library-transition");ghost.classList.remove("is-dragging","is-drop-target");ghost.setAttribute("aria-hidden","true");ghost.inert=true;root.append(ghost);
    if(target){const x=(parseFloat(target.style.left)||0)-old.left,y=(parseFloat(target.style.top)||0)-old.top;
      animate(ghost,[{transform:"none",opacity:1},{offset:.6,transform:'translate('+x*.7+'px,'+(y*.7-20)+'px) scale(.65) rotate(-7deg)',opacity:1},{transform:'translate('+x+'px,'+y+'px) scale(.12)',opacity:0}],0,true);
    }else animate(ghost,[{transform:"scale(1)",opacity:1},{transform:"scale(.12)",opacity:0}],0,true);
  }
  return true;
}
function libraryDrop(source,target,flight=null){return animateLibraryChange(()=>library.move(source,target),flight,"collect");}
function liftLibraryCard(drag){
  const rect=drag.row.getBoundingClientRect(),ghost=drag.row.cloneNode(true);ghost.classList.remove("is-dragging","is-drop-target");ghost.classList.add("library-drag-preview");ghost.inert=true;ghost.setAttribute("aria-hidden","true");Object.assign(ghost.style,{position:"fixed",left:rect.left+"px",top:rect.top+"px",width:(rect.width||parseFloat(drag.row.style.width)||132)+"px",height:(rect.height||FILE_HEIGHT)+"px",right:"auto"});document.body.append(ghost);drag.ghost=ghost;
}
function endLibraryDrag(event){if(!libraryDrag)return;const drag=libraryDrag;libraryDrag=null;drag.row.classList.remove("is-dragging");document.querySelectorAll(".is-drop-target").forEach(row=>row.classList.remove("is-drop-target"));if(drag.active){suppressLibraryClick=Date.now()+MOTION_MS;let grouped=false;if(event.type==="pointerup"&&drag.target)grouped=libraryDrop(drag.key,drag.target,drag);
  if(!grouped&&drag.ghost?.animate){const animation=drag.ghost.animate([{transform:drag.ghost.style.transform},{transform:"none"}],{duration:MOTION_MS,easing:"cubic-bezier(.22,.8,.2,1)"});animation.finished.then(()=>drag.ghost.remove(),()=>drag.ghost.remove());}else drag.ghost?.remove();}}
document.addEventListener("pointermove",event=>{const drag=libraryDrag;if(!drag||event.pointerId!==drag.id)return;if(!drag.active&&Math.hypot(event.clientX-drag.x,event.clientY-drag.y)>=7){drag.active=true;liftLibraryCard(drag);drag.row.classList.add("is-dragging");}if(!drag.active)return;event.preventDefault();drag.ghost.style.transform='translate3d('+(event.clientX-drag.x)+'px,'+(event.clientY-drag.y)+'px,0) rotate(-4deg) scale(.92)';const target=document.elementFromPoint?.(event.clientX,event.clientY)?.closest(".library-tile");const key=target?.dataset.nodeKey;drag.target=key&&key!==drag.key?key:null;document.querySelectorAll(".is-drop-target").forEach(row=>row.classList.remove("is-drop-target"));if(drag.target)target.classList.add("is-drop-target");});
document.addEventListener("pointerup",endLibraryDrag);document.addEventListener("pointercancel",endLibraryDrag);window.addEventListener("blur",()=>endLibraryDrag({type:"cancel"}));
$("file-panel").addEventListener("scroll",()=>{if(listFrame!==null)return;listFrame=window.requestAnimationFrame(()=>{listFrame=null;renderVirtualQueue();});});
new ResizeObserver(()=>{if(!$("file-panel").hidden&&!libraryPanelAnimation)renderVirtualQueue();}).observe($("file-panel"));
function updatePreviewBackground(){if(vectorBitmap)paintVector(vectorBitmap.image,vectorBitmap.geometry,vectorBitmap.native);}
function renderPreview() {
  const image = current(); $("empty").hidden = Boolean(image); $("previews").hidden = !image;
  $("result-parameters").hidden = !image?.result;
  if (!image) { previewId=null; clearVector(); clearOriginal(); return; }
  if (previewId !== image.id) { previewId = image.id; scale = null; pans.clear(); clearOriginal(); }
  if(image.preview)$("overlay-original").src=image.preview;else{$("overlay-original").removeAttribute("src");ensurePreview(image.id);}
  $("result-parameters").textContent = resultParameters(image);
  $("result-parameters").removeAttribute("title");
  $("result-parameters").hidden = !image.result;
  $("original-size").textContent = `${image.pdfGroup?"PDF ":""}${formatBytes(image.bytes)}`;
  $("vector-size").textContent = image.result ? formatBytes(image.result.bytes) : "";
  if (image.result) loadVector(image);
  else clearVector();
  zoom();
}
function render() { renderQueue(); renderPreview(); updateButtons(); }
function invalidate() {
  revision++; tracePending=Boolean(current());
  let settings;try{settings=options();rememberFileOptions(settings);}catch(error){tracePending=false;status(error.message,true);}
  invoke?.("cancel_trace",{revision,id:selected,options:settings||null}).catch(()=>{});
  renderQueue();
  if(traceTimer!==null)clearTimeout(traceTimer);
  traceTimer=setTimeout(()=>{traceTimer=null;runTrace();},90);
  updateButtons();
}
async function runTrace() {
  if(tracing || busy || !tracePending || !invoke)return;
  const image=current(), token=revision;
  if(!image){tracePending=false;updateButtons();return;}
  let settings;try{settings=options();}catch(error){tracePending=false;status(error.message,true);updateButtons();return;}
  tracePending=false;tracing=true;updateButtons();
  try {
    const result=await invoke("trace_image",{id:image.id,revision:token,options:settings});
    if(token===revision && selected===image.id){
      discardPreview();live={id:image.id,revision:token,result,resultOptions:{...settings},url:`preview:${image.id}:${token}`};

      status("");renderQueue();renderPreview();
    }
  }catch(error){if(token===revision && selected===image.id){if(!String(error).includes("描摹已取消"))status(String(error),true);}}
  finally{tracing=false;updateButtons();if(tracePending && traceTimer===null)runTrace();}
}
async function importImages(paths=null) {
  if (busy || !invoke) return;
  const inherited=options();busy=true;updateButtons();
  try {
    const result=await invoke("import_images",{paths});
    if(result.images.length){importEpoch++;thumbCache.clear();queueChanged();}
    for(const image of result.images){const key=sourceKey(image);if(!fileOptions.has(key))fileOptions.set(key,{...inherited});const index=images.findIndex(entry=>entry.id===image.id);if(index<0)images.push(image);else images[index]=image;}
    if(result.images.length){library.normalize(images);syncLibrary();selected=result.images[0].id;loadFileOptions(result.images[0]);fullPreview=null;discardPreview();invalidate();setFilePanel(false);}
    status(result.errors.length?result.errors.join("；"):"",result.errors.length>0);
  }catch(error){status(String(error),true);}
  finally{busy=false;render();if(tracePending && traceTimer===null)runTrace();}
}

$("workspace").addEventListener("click",event=>{if(!images.length && !event.target.closest("button, #value-popover, #notification-panel, #notification-toast, .parameter-toolbar"))importImages();});
$("empty").addEventListener("keydown",event=>{if(!images.length && ["Enter"," "].includes(event.key)){event.preventDefault();importImages();}});
const webview=window.__TAURI__?.webview?.getCurrentWebview();
webview?.onDragDropEvent(event=>{
  const {type,paths}=event.payload;
  $("workspace").classList.toggle("drop-hover",type==="enter" || type==="over");
  if(type==="drop" && paths?.length)importImages(paths);
}).catch(error=>status(`无法监听拖入图片：${error}`,true));
function savedMessage(path){
  const name=String(path).split(/[\\/]/).pop().replace(/\.[^.]+$/, ""),characters=Array.from(name);
  const shortened=characters.length>32?`${characters.slice(0,20).join("")}...${characters.slice(-9).join("")}`:name;
  return `“${shortened}”已保存成功`;
}
async function saveCurrent(saveAs=false){
  if($("save").disabled||!live)return;const snapshot=live;busy=true;updateButtons();
  try{const saved=await invoke("save_image",{id:snapshot.id,options:snapshot.resultOptions,svg:snapshot.result.svg,saveAs});status(saved?savedMessage(saved):"",false,saved);}
  catch(error){status(String(error),true);}finally{busy=false;render();if(tracePending&&traceTimer===null)runTrace();}
}
$("save").addEventListener("click",()=>saveCurrent(false));
$("vcPreset").addEventListener("change",()=>{const p={default:[.4,.4,0,.4,false],logo:[.65,.65,8,.4,false],lineart:[.6,.6,4,.4,false],poster:[.5,.5,12,.4,false],photo:[.4,.4,0,.4,true],detailed:[.3,.3,2,.4,false],clean:[.4,.4,4,.75,false]}[$("vcPreset").value];for(const [i,id]of ["vcTolerance","vcFitError","vcMinArea","vcOptimizeError","vcGradients"].entries()){if($(id).type==="checkbox")$(id).checked=p[i];else $(id).value=p[i];}});
for (const id of fields) $(id).addEventListener("change",id==="transparency"?()=>{
  updatePreviewBackground();refreshControls();try{const settings=options();rememberFileOptions(settings);if(live)live.resultOptions.transparency=settings.transparency;invoke?.("update_preview_setting",{id:selected,options:settings,revision}).catch(error=>status(String(error),true));}catch(error){status(String(error),true);}
}:invalidate);
function zoom() {
  const image = current(); if (!image) return;
  for (const canvas of document.querySelectorAll(".canvas")) {
    if (!canvas.clientWidth || !canvas.clientHeight) continue;
    const ratio = scale ?? Math.min(canvas.clientWidth / image.width, canvas.clientHeight / image.height);
    const stage = canvas.querySelector(".image-stage");
    const width = Math.max(1, image.width * ratio), height = Math.max(1, image.height * ratio);
    const pan = pans.get(canvas.id) || { x: 0, y: 0 };
    pan.x = Math.max(-Math.max(0,(width-canvas.clientWidth)/2),Math.min(Math.max(0,(width-canvas.clientWidth)/2),pan.x));
    pan.y = Math.max(-Math.max(0,(height-canvas.clientHeight)/2),Math.min(Math.max(0,(height-canvas.clientHeight)/2),pan.y));
    pans.set(canvas.id,pan);
    stage.style.width = `${width}px`; stage.style.height = `${height}px`;
    stage.style.left = `${(canvas.clientWidth-width)/2+pan.x}px`; stage.style.top = `${(canvas.clientHeight-height)/2+pan.y}px`;
    if (canvas.id === "overlay-canvas" && !$("overlay-pane").hidden) {
      const rect = canvas.getBoundingClientRect();
      overlayGeometry = { left:rect.left, top:rect.top, width:canvas.clientWidth, height:canvas.clientHeight, stageLeft:parseFloat(stage.style.left), stageTop:parseFloat(stage.style.top), stageWidth:width, stageHeight:height };
    }
  }
  transformVector();
  transformOriginal();
  scheduleOriginal();
  scheduleRaster();
  updateSplit(split);
}
function clearVector() {
  if (rasterTimer !== null) clearTimeout(rasterTimer);
  rasterTimer = null; vectorLoad++; vectorSource = null; vectorSourceKey = null; vectorRaster = null; vectorBitmap = null;
  $("vector-layer").hidden = true;
}
async function loadVector(image) {
  if (vectorSourceKey === image.url) return;
  clearVector(); vectorSourceKey = image.url;
  const token = ++vectorLoad, source = new Image(); source.decoding = "async";
  if(!image.result.preview)return;
  source.src = image.result.preview;
  try {
    await source.decode();
    if (token !== vectorLoad || current()?.url !== image.url) return;
    vectorSource = source; drawVector(); updateSplit(split);
    // The small trace bitmap is only an immediate fallback. Once it is decoded,
    // render the vector scene at the current viewport resolution without waiting for a zoom.
    if (rasterTimer !== null) clearTimeout(rasterTimer);
    rasterTimer = null;
    void requestRaster();
  } catch (error) {
    if (token === vectorLoad) status(`无法显示矢量预览：${error.message || error}`, true);
  }
}
function drawVector() {
  if (!vectorSource || !overlayGeometry || vectorSourceKey !== current()?.url) return;
  paintVector(vectorSource,{...overlayGeometry},false);
}
function paintVector(bitmap,g,native){
  const canvas=$("overlay-result"),margin=RASTER_MARGIN,width=g.width+margin*2,height=g.height+margin*2;
  const ratio=Math.min(window.devicePixelRatio||1,Math.sqrt(RASTER_PIXELS/(width*height)));
  const context=canvas.getContext("2d",{alpha:true});if(!context)return;
  canvas.width=Math.max(1,Math.floor(width*ratio));canvas.height=Math.max(1,Math.floor(height*ratio));
  canvas.style.width=width+"px";canvas.style.height=height+"px";canvas.style.left=-margin+"px";canvas.style.top=-margin+"px";
  context.setTransform(ratio,0,0,ratio,0,0);context.clearRect(0,0,width,height);
  // The backing belongs exclusively to the clipped vector layer; input pixels and exported SVG stay unchanged.
  if($("transparency").value==="white"){context.fillStyle="#fff";context.fillRect(g.stageLeft+margin,g.stageTop+margin,g.stageWidth,g.stageHeight);}
  if(native){context.setTransform(1,0,0,1,0,0);context.drawImage(bitmap,0,0);}
  else context.drawImage(bitmap,g.stageLeft+margin,g.stageTop+margin,g.stageWidth,g.stageHeight);
  vectorBitmap={image:bitmap,geometry:g,native};vectorRaster=g;$("vector-layer").hidden=false;transformVector();
}
async function requestRaster(){const image=current();if(!image?.result||!vectorSource||!overlayGeometry||!invoke)return;const request=++rasterRequest,token=vectorLoad,key=vectorSourceKey,g={...overlayGeometry},margin=RASTER_MARGIN,width=g.width+margin*2,height=g.height+margin*2,ratio=Math.min(window.devicePixelRatio||1,Math.sqrt(RASTER_PIXELS/(width*height)));try{const url=await invoke("raster_preview",{revision:live.revision,request,geometry:{width:Math.max(1,Math.floor(width*ratio)),height:Math.max(1,Math.floor(height*ratio)),x:(g.stageLeft+margin)*ratio,y:(g.stageTop+margin)*ratio,stageWidth:g.stageWidth*ratio,stageHeight:g.stageHeight*ratio}});if(!url||request!==rasterRequest||token!==vectorLoad||key!==current()?.url)return;const bitmap=new Image();bitmap.src=url;await bitmap.decode();if(request!==rasterRequest||token!==vectorLoad||key!==current()?.url)return;paintVector(bitmap,g,true);updateSplit(split);}catch(error){if(token===vectorLoad&&request===rasterRequest&&!String(error).includes("描摹已取消"))status(`无法刷新矢量预览：${error}`,true);}}
function transformVector() {
  if (!vectorRaster || !overlayGeometry) return;
  const g=overlayGeometry, previous=vectorRaster, factor=g.stageWidth/previous.stageWidth;
  const x=g.stageLeft-previous.stageLeft*factor+RASTER_MARGIN*(1-factor);
  const y=g.stageTop-previous.stageTop*factor+RASTER_MARGIN*(1-factor);
  $("overlay-result").style.transform = `translate3d(${x}px,${y}px,0) scale(${factor})`;
}
function scheduleRaster() {
  if (!vectorSource) return;
  if (rasterTimer !== null) clearTimeout(rasterTimer);
  rasterTimer = null;
  if(vectorBitmap?.native && vectorRaster && overlayGeometry && ["width","height","stageLeft","stageTop","stageWidth","stageHeight"].every(key=>vectorRaster[key]===overlayGeometry[key]))return;
  rasterTimer = setTimeout(() => { rasterTimer=null;requestRaster(); },120);
}
function clearOriginal(){if(originalTimer!==null)clearTimeout(originalTimer);originalTimer=null;originalRequest++;originalRaster=null;$("original-layer").hidden=true;$("overlay-original").style.visibility="";}
function transformOriginal(){if(!originalRaster||!overlayGeometry)return;const g=overlayGeometry,p=originalRaster,f=g.stageWidth/p.stageWidth;$("original-raster").style.transform=`translate3d(${g.stageLeft-p.stageLeft*f+RASTER_MARGIN*(1-f)}px,${g.stageTop-p.stageTop*f+RASTER_MARGIN*(1-f)}px,0) scale(${f})`;}
function scheduleOriginal(){if(!invoke||!overlayGeometry)return;if(originalTimer!==null)clearTimeout(originalTimer);originalTimer=null;
 if(originalRaster&&["width","height","stageLeft","stageTop","stageWidth","stageHeight"].every(k=>originalRaster[k]===overlayGeometry[k]))return;
 originalTimer=setTimeout(()=>{originalTimer=null;requestOriginal();},120);
}
async function requestOriginal(){const image=current();if(!image||!overlayGeometry||!invoke)return;const id=image.id,request=++originalRequest,g={...overlayGeometry},margin=RASTER_MARGIN,width=g.width+2*margin,height=g.height+2*margin,ratio=Math.min(window.devicePixelRatio||1,Math.sqrt(RASTER_PIXELS/(width*height)));
 try{const url=await invoke("original_preview",{id,request,geometry:{width:Math.max(1,Math.floor(width*ratio)),height:Math.max(1,Math.floor(height*ratio)),x:(g.stageLeft+margin)*ratio,y:(g.stageTop+margin)*ratio,stageWidth:g.stageWidth*ratio,stageHeight:g.stageHeight*ratio}});
 if(!url||request!==originalRequest||id!==selected)return;const bitmap=new Image();bitmap.src=url;await bitmap.decode();if(request!==originalRequest||id!==selected)return;
 const canvas=$("original-raster"),context=canvas.getContext("2d",{alpha:true});if(!context)return;canvas.width=Math.max(1,Math.floor(width*ratio));canvas.height=Math.max(1,Math.floor(height*ratio));canvas.style.width=`${width}px`;canvas.style.height=`${height}px`;canvas.style.left=`${-margin}px`;canvas.style.top=`${-margin}px`;context.setTransform(1,0,0,1,0,0);context.clearRect(0,0,canvas.width,canvas.height);context.drawImage(bitmap,0,0);originalRaster=g;$("original-layer").hidden=false;$("overlay-original").style.visibility="hidden";transformOriginal();updateSplit(split);
 }catch(error){if(request===originalRequest&&id===selected&&!String(error).includes("描摹已取消"))status(`无法刷新原图预览：${error}`,true);}
}
function fit() { scale = null; pans.clear(); clearOriginal(); zoom(); }
function toggleFit(){const image=current(),canvas=$("overlay-canvas");if(!image)return;const ratio=Math.min(canvas.clientWidth/image.width,canvas.clientHeight/image.height),atFit=scale===null||(Math.abs(scale-ratio)<1e-6&&[...pans.values()].every(p=>Math.abs(p.x)<.1&&Math.abs(p.y)<.1));scale=atFit?1:null;pans.clear();clearOriginal();zoom();}
function updateSplit(value) {
  if (!current()?.result || !vectorSource || vectorSourceKey !== current()?.url) {
    $("overlay-original").style.clipPath = "";
    $("original-layer").style.clipPath = "";
    for (const id of ["wipe-line", "original-size", "vector-size"]) $(id).hidden = true;
    return;
  }
  split = Math.max(0, Math.min(1, value));
  const canvas = $("overlay-canvas"), geometry = overlayGeometry;
  const cut = split === 0 || split === 1 || !geometry?.stageWidth ? split : Math.max(0, Math.min(1, (split * geometry.width - geometry.stageLeft) / geometry.stageWidth));
  $("overlay-original").style.clipPath = `inset(0 ${(1 - cut) * 100}% 0 0)`;
  $("vector-layer").style.clipPath = `inset(0 0 0 ${split * 100}%)`;
  $("original-layer").style.clipPath = `inset(0 ${(1-split)*100}% 0 0)`;
  const lineX = Math.max(0,Math.min(canvas.clientWidth,split*canvas.clientWidth));
  $("wipe-line").style.left = `${Math.max(0,Math.min(canvas.clientWidth-2,lineX))}px`;
  $("wipe-line").style.top = "0px"; $("wipe-line").style.height = `${canvas.clientHeight}px`;
  $("wipe-line").hidden = false;
  for (const id of ["original-size","vector-size"]) { $(id).style.top = `${Math.max(18,Math.min(canvas.clientHeight-18,pointerHeight*canvas.clientHeight))}px`; }
  $("original-size").style.left = `${lineX-10}px`; $("vector-size").style.left = `${lineX+10}px`;
  $("original-size").hidden = split === 0; $("vector-size").hidden = split === 1;
  $("overlay-canvas").setAttribute("aria-valuenow", String(Math.round(split * 100)));
}
function flushWipe() {
  wipeFrame = null;
  if (!wipePointer || !overlayGeometry || dragging || $("overlay-pane").hidden) return;
  pointerHeight = Math.max(0,Math.min(1,(wipePointer.y-overlayGeometry.top)/overlayGeometry.height));
  updateSplit((wipePointer.x-overlayGeometry.left)/overlayGeometry.width);
}
function followWipe(event, immediate = false) {
  if (!current()?.result || dragging || !overlayGeometry) return;
  const events = event.getCoalescedEvents?.();
  const latest = events?.length ? events[events.length-1] : event;
  wipePointer = {x:latest.clientX,y:latest.clientY};
  if (immediate || !window.requestAnimationFrame) {
    if (wipeFrame !== null) window.cancelAnimationFrame?.(wipeFrame);
    flushWipe();
  } else if (wipeFrame === null) wipeFrame = window.requestAnimationFrame(flushWipe);
}
document.addEventListener("pointermove", followWipe);
$("overlay-canvas").addEventListener("pointerleave", event => followWipe(event, true));
$("overlay-canvas").addEventListener("keydown", (event) => { if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) { event.preventDefault(); updateSplit(event.key === "Home" ? 0 : event.key === "End" ? 1 : split + (event.key === "ArrowRight" ? .05 : -.05)); } });
for (const canvas of document.querySelectorAll(".canvas")) canvas.addEventListener("wheel", (event) => {
  const image = current(); if (!image) return; event.preventDefault();
  const stage = canvas.querySelector(".image-stage");
  const before = stage.getBoundingClientRect();
  const delta = event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? canvas.clientHeight : 1);
  scale = Math.max(.02, Math.min(16, (scale ?? before.width / image.width) * Math.exp(-delta * .0015)));
  const factor=scale/(before.width/image.width); for (const pan of pans.values()) { pan.x*=factor;pan.y*=factor; }
  zoom();
  updateSplit(split);
}, { passive: false });
new ResizeObserver(() => zoom()).observe($("previews"));
function setFilePanel(open){animatePanel("file-panel",open);$("toggle-files").setAttribute("aria-expanded",String(open));if(open){renderQueue();const entry=libraryLayout.entries.find(({item})=>item.image?.id===selected&&item.type==="image");if(entry){const panel=$("file-panel"),height=panel.clientHeight||600,top=Math.floor(entry.slot/libraryColumns)*(FILE_HEIGHT+LIBRARY_GAP)+$("files").offsetTop;if(top<panel.scrollTop||top+FILE_HEIGHT>panel.scrollTop+height)panel.scrollTop=Math.max(0,top-(height-FILE_HEIGHT)/2);}renderVirtualQueue();}else{thumbnailObserver?.disconnect();visibleThumbs.clear();}}
$("library-add").addEventListener("click",()=>importImages());
$("library-clear").addEventListener("click",()=>removeImages(images.map(image=>image.id)));
$("toggle-files").addEventListener("click", () => setFilePanel(!panelStates.get("file-panel")?.open));
document.addEventListener("pointerdown", (event) => { if(!event.target.closest("#notifications, #notification-panel, #notification-toast"))setNotifications(false); if (!event.target.closest("#file-panel, #toggle-files")) setFilePanel(false); if (!event.target.closest("#value-popover, .parameter-button")) $("value-popover").hidden = true; });

for (const action of ["minimize", "maximize", "close"]) $(action).addEventListener("click", async () => { if (!invoke) return; try { if(action==="close"){let settings=null;try{settings=options();}catch{}await librarySync;await invoke("save_session",{id:selected,options:settings,revision});} const maximized = await invoke("window_action", { action }); if (action === "maximize") $("maximize").innerHTML = `<svg viewBox="0 0 24 24" aria-hidden="true">${maximized?'<path d="M9 6h9v9M6 9h9v9H6z"/>':'<rect x="6" y="6" width="12" height="12" rx="1"/>'}</svg>`; } catch(error) { status(String(error), true); } });
let logoGesture=null,suppressLogoClick=false;
const logoButton=$("background-logo");
function toggleBackground(){if($("transparency").disabled)return;$("transparency").value=$("transparency").value==="off"?"white":"off";$("transparency").dispatchEvent(new Event("change"));}
function dragLogo(){if(!logoGesture||logoGesture.dragged)return;clearTimeout(logoGesture.timer);logoGesture.dragged=true;suppressLogoClick=true;hideTooltip();invoke?.("window_action",{action:"drag"}).catch(error=>status(String(error),true));}
logoButton.addEventListener("pointerdown",event=>{if(event.button!==0)return;suppressLogoClick=false;logoGesture={id:event.pointerId,x:event.clientX,y:event.clientY,dragged:false,timer:setTimeout(dragLogo,220)};});
document.addEventListener("pointermove",event=>{if(logoGesture&&event.pointerId===logoGesture.id&&Math.hypot(event.clientX-logoGesture.x,event.clientY-logoGesture.y)>=4)dragLogo();});
function releaseLogo(){if(!logoGesture)return;clearTimeout(logoGesture.timer);logoGesture=null;}
document.addEventListener("pointerup",releaseLogo);document.addEventListener("pointercancel",releaseLogo);window.addEventListener("blur",releaseLogo);
logoButton.addEventListener("click",event=>{if(suppressLogoClick&&event.detail!==0){suppressLogoClick=false;return;}toggleBackground();});
logoButton.addEventListener("wheel",event=>{if(event.shiftKey||Math.abs(event.deltaX)>Math.abs(event.deltaY))return;event.preventDefault();event.stopPropagation();if(event.deltaY)toggleBackground();},{passive:false});
$("titlebar").addEventListener("mousedown", (event) => { if (event.button === 0 && !event.target.closest("button") && invoke) invoke("window_action", { action: "drag" }).catch(error => status(String(error), true)); });
$("titlebar").addEventListener("dblclick", (event) => { if (!event.target.closest("button")) $("maximize").click(); });
const hints = {
 dither: "用误差扩散模拟减色后丢失的颜色层次。关闭更适合字帖和清晰轮廓；Floyd–Steinberg 按行扩散误差；Riemersma 沿连续曲线扩散，颗粒分布不同。拟色可能增加路径数量和文件大小。",
 transparency: "只为描摹结果的透明和半透明区域添加白色预览衬底。关闭时显示棋盘格；开启时显示白底。原图不变，不删除颜色，也不改变导出文件的透明度。",
 colors: "描摹使用的颜色数量，1–32。1 是单色，透明区域为背景；2–32 为彩色。颜色越多通常越慢、SVG 越大。",
 quantization: "Median cut 保持原方案；Vecline Lloyd 使用 RGB Wu 初始化及分桶 Oklab 精修；Oklab Wu + Lloyd 使用 Oklab Wu 初始化及完整颜色加权迭代。两者均可用 0 次迭代仅运行初始化。单色时 Vecline 使用亮度阈值，另两者训练两类颜色后提取深色前景。",
 prescale: "在描摹之前缩放输入位图，最低 0.5；1 表示不变，低于 1 每档 0.1，1 及以上每档 0.5。放大可能保留细线，缩小可减少噪点和计算量；不增加真实细节。输出仍是矢量图，默认显示尺寸不变。",
 despeckle: "删除面积不超过此像素数的斑点和小孔。0 保留全部；数值过大可能删除标点或细节。像素以描摹精度后的位图为准。",
 smoothCorners: "决定哪些转角保留为尖角、哪些拟合为平滑曲线。0 全部保留为角点，跳过平滑判定和路径合并；数值增大更圆滑，最多 1.3，每档 0.05。",
 optimizePaths: "相邻平滑贝塞尔曲线的合并误差容限，单位为描摹内部位图的像素。默认 5，每档 0.5；0 跳过合并。数值越大越容易合并、减少曲线段与 SVG 大小，也可能改变细节。描摹精度越高，同一容限对应的原图偏差越小；这不是整条轮廓误差的严格保证。转角平滑为 0 或轮廓全为角点时自动跳过合并。",

};
const optionHints = {
 quantization: { mc:"Median cut：保留原减色方案。",vc:"Vecline Lloyd：RGB Wu 初始化，分桶 Oklab 精修。",wl:"Oklab Wu + Lloyd：感知颜色空间初始化，完整颜色分布迭代。" },

};
Object.assign(hints,{veclineIterations:"Vecline 平均色的分桶 Lloyd 迭代次数，0–32；0 跳过。主导色和中位色不精修。"});
Object.assign(hints,Object.fromEntries(algorithmParameters.map(def=>[def.id,def.hint])),{tracing:"选择边界算法。Potrace 逐色二值化并叠加；Vecline 提取彩色连通区域，拟合共享或独立边界，可重建渐变。两者都可配合三种减色算法。"});
for (const select of document.querySelectorAll("select")) for (const option of select.options) option.title = optionHints[select.id]?.[option.value] || hints[select.id] || "";
for (const [id, hint] of Object.entries(hints)) { $(id).title = hint; const label = $(id).closest("label"); if (label) label.title = hint; }
const names = {colors:"色数",quantization:"减色算法",prescale:"描摹精度",despeckle:"斑点过滤",smoothCorners:"转角平滑",optimizePaths:"路径优化",transparency:"描摹结果白底预览",dither:"拟色",veclineIterations:"精修次数"};
const steps = {colors:1,prescale:.5,despeckle:1,smoothCorners:.05,optimizePaths:.5,veclineIterations:1};
function positionValuePopover(button){const rect=button.getBoundingClientRect(),popover=$("value-popover"),width=popover.offsetWidth||250;popover.style.left=`${Math.max(width/2+8,Math.min(window.innerWidth-width/2-8,rect.left+rect.width/2))}px`;popover.style.top=`${rect.top-10}px`;}
Object.assign(names,Object.fromEntries(algorithmParameters.map(def=>[def.id,def.name])),{tracing:"描摹方式"});
Object.assign(steps,Object.fromEntries(algorithmParameters.filter(def=>def.type==="number").map(def=>[def.id,def.step])));
const svg = (body) => `<svg viewBox="0 0 24 24" aria-hidden="true">${body}</svg>`;
const offIcon=svg('<circle cx="12" cy="12" r="8"/><path d="M6 18L18 6"/>');
const icons = {
 colors:svg('<path d="M12 3a9 9 0 1 0 0 18h2a2 2 0 0 0 0-4h-1a2 2 0 0 1 0-4h4a4 4 0 0 0 4-4c0-4-5-6-9-6z"/><circle cx="7" cy="9" r="1"/><circle cx="11" cy="6" r="1"/><circle cx="16" cy="7" r="1"/>'),
 prescale:svg('<circle cx="10" cy="10" r="6"/><path d="M14.5 14.5L20 20M8 8h4v4H8zM10 8v4M8 10h4"/>'),
 despeckle:svg('<circle cx="8" cy="8" r="4"/><circle cx="17" cy="16" r="2"/><path d="M4 20L20 4"/>'),
 smoothCorners:svg('<path d="M4 20V10a6 6 0 0 1 6-6h10M4 4v4M16 20h4v-4"/>'),
 optimizePaths:svg('<path d="M4 18C8 18 10 4 20 6"/><circle cx="4" cy="18" r="2"/><circle cx="20" cy="6" r="2"/>'),

 quantization:[svg('<path d="M5 19V11M10 19V5M15 19V8M20 19V13"/>'),svg('<path d="M4 4h16v16H4zM12 4v16M4 12h16"/>'),svg('<circle cx="5" cy="12" r="2"/><circle cx="16" cy="5" r="2"/><circle cx="18" cy="18" r="2"/><path d="M7 11l7-5M7 13l9 4M17 7l1 9"/>')],
 dither:[offIcon,svg('<path d="M4 5h4M4 10h8M4 15h12M4 20h16M16 4v4M20 9v4"/>'),svg('<path d="M4 4v16h5V9h6v11h5V4h-5v5H9V4z"/>')]
};
icons.veclineIterations=svg('<path d="M19 8a8 8 0 1 0 1 8M19 3v5h-5M8 12h8M12 8v8"/>');
icons.tracing=[svg('<path d="M4 20V8h4V4h12v12h-4v4z"/>'),svg('<path d="M4 18C5 5 13 3 20 7M4 18C8 21 18 19 20 7"/><circle cx="4" cy="18" r="2"/><circle cx="20" cy="7" r="2"/>')];
const algorithmGlyphs={
  "lloydIterations": "<path d=\"M4 4h16v16H4zM8 4v16M16 4v16M4 8h16M4 16h16\"/>",
  "monoCutoff": "<path d=\"M4 17h16M4 13l5-7 5 7 6-9M13 4v16\"/>",
  "vcTolerance": "<path d=\"M3 17l4-6 5 4 5-9 4 3M3 20h18M7 19v2M17 19v2\"/>",
  "vcFitError": "<path d=\"M4 18C7 2 17 2 20 18M5 12h14M12 10v4\"/>",
  "vcOptimizeError": "<path d=\"M4 6h16M4 18h16M6 10h12M6 14h12M12 7v10M10 9l2-2 2 2M10 15l2 2 2-2\"/>",
  "vcMinArea": "<circle cx=\"12\" cy=\"12\" r=\"7\"/><path d=\"M8 12h8M12 8v8M3 4h1M20 19h1M20 4h1\"/>",
  "vcGradients": "<rect x=\"4\" y=\"4\" width=\"16\" height=\"16\" rx=\"2\"/><path d=\"M7 6v12M10 7v10M13 8v8M16 9v6\"/>"
};
for(const def of algorithmParameters){if(def.type!=="select")icons[def.id]=svg(algorithmGlyphs[def.id]);}
icons.vcMode=algorithmParameters.find(d=>d.id==="vcMode").values.map((_,i)=>svg(['<path d="M3 18C5 2 19 22 21 6"/>','<path d="M4 4h16v16H4zM8 8l8 8M16 8l-8 8"/>','<path d="M3 3h8v8H3zM13 3h8v8h-8zM3 13h8v8H3zM13 13h8v8h-8z"/>','<path d="M3 18h5v-5h5V8h8M3 18h18V3"/>'][i]));
icons.vcPreset=algorithmParameters.find(d=>d.id==="vcPreset").values.map((_,i)=>svg('<path d="M4 5h16v14H4zM4 15l5-5 4 4 3-2 4 5"/><circle cx="16" cy="8" r="1"/>'));
icons.monoThreshold=[svg('<path d="M4 19V9M8 19V5M12 19V8M16 19v-7M20 19v-3M3 13h18"/>'),svg('<path d="M4 5h16M4 19h16M12 5v14M8 12h8"/>'),svg('<rect x="4" y="4" width="16" height="16" rx="2"/><path d="M8 16l4-8 4 8M6 12h12"/>')];
let tooltipOwner=null,tooltipTimer=null;
const tooltip=document.createElement("div");tooltip.id="control-tooltip";tooltip.className="control-tooltip";tooltip.setAttribute("role","tooltip");tooltip.hidden=true;document.body.append(tooltip);
function hideTooltip(){if(tooltipTimer!==null)clearTimeout(tooltipTimer);tooltipTimer=null;tooltipOwner?.removeAttribute("aria-describedby");tooltipOwner=null;tooltip.hidden=true;}
function paintTooltip(button){
  const text=button.dataset.tooltip||"",parts=text.split("\n"),heading=document.createElement("div");heading.className="tooltip-heading";heading.textContent=parts.shift();tooltip.replaceChildren(heading);
  const id=button.dataset.field,input=id?$(id):null;
  if(input?.tagName==="SELECT")for(const option of input.options){const line=document.createElement("div");line.className="tooltip-option";line.textContent=`${option.selected?"●":"○"} ${option.textContent}`;tooltip.append(line);}
  for(const paragraph of parts.filter(Boolean)){for(const sentence of paragraph.split("。").filter(Boolean)){const p=document.createElement("p");if(paragraph.startsWith("主页："))p.className="tooltip-homepage";p.textContent=sentence+(paragraph.includes("。")?"。":"");tooltip.append(p);}}
  tooltip.classList.toggle("single-line",tooltip.children.length===1);
  const rect=button.getBoundingClientRect(),wasHidden=tooltip.hidden,logoHint=button.id==="background-logo";tooltip.classList.toggle("logo-tooltip",logoHint);tooltip.style.maxWidth=`${Math.min(logoHint?460:264,window.innerWidth-20)}px`;tooltip.style.visibility="hidden";tooltip.hidden=false;const width=tooltip.getBoundingClientRect().width||tooltip.offsetWidth||Math.min(264,window.innerWidth-20);tooltip.style.left=`${Math.max(10,Math.min(window.innerWidth-width-10,rect.left+rect.width/2-width/2))}px`;const isLogo=Boolean(button.closest("#titlebar"));tooltip.style.transform=isLogo?"none":"translateY(-100%)";tooltip.style.top=`${isLogo?rect.bottom+12:rect.top-12}px`;tooltip.style.maxHeight=`${Math.max(40,isLogo?window.innerHeight-rect.bottom-22:rect.top-22)}px`;tooltip.hidden=wasHidden;tooltip.style.visibility="";
}
function showTooltip(button){hideTooltip();tooltipOwner=button;tooltipTimer=setTimeout(()=>{if(tooltipOwner!==button)return;paintTooltip(button);tooltip.hidden=false;button.setAttribute("aria-describedby",tooltip.id);},220);}
for(const button of document.querySelectorAll(".parameter-button,.round-button:not(#notifications),.window-actions button,#background-logo")){
  if(!button.dataset.field)button.dataset.tooltip=button.title;button.removeAttribute("title");
  button.addEventListener("mouseenter",()=>showTooltip(button));button.addEventListener("mouseleave",hideTooltip);button.addEventListener("focus",()=>showTooltip(button));button.addEventListener("blur",hideTooltip);button.addEventListener("pointerdown",hideTooltip);
}
window.addEventListener("blur",hideTooltip);window.addEventListener("resize",()=>{hideTooltip();$("value-popover").hidden=true;});$("file-panel").addEventListener("scroll",hideTooltip);
function enablePointerGlow(surface){
  surface.classList.add("pointer-surface");if(getComputedStyle(surface).position==="static")surface.style.position="relative";
  let frame=null,x=0,y=0;
  const paint=()=>{frame=null;const rect=surface.getBoundingClientRect();surface.style.setProperty("--pointer-x",(x-rect.left)+"px");surface.style.setProperty("--pointer-y",(y-rect.top)+"px");surface.style.setProperty("--glow-scroll",surface.scrollTop+"px");};
  surface.addEventListener("pointermove",event=>{x=event.clientX;y=event.clientY;surface.style.setProperty("--pointer-active",1);if(frame===null)frame=requestAnimationFrame(paint);});
  surface.addEventListener("scroll",()=>{if(surface.style.getPropertyValue("--pointer-active")==="1"&&frame===null)frame=requestAnimationFrame(paint);},{passive:true});
  surface.addEventListener("pointerleave",()=>{surface.style.setProperty("--pointer-active",0);if(frame!==null)cancelAnimationFrame(frame);frame=null;});
  window.addEventListener("blur",()=>surface.style.setProperty("--pointer-active",0));
}
for(const surface of document.querySelectorAll(".toolbar-group,.files-trigger,.window-actions button,#background-logo,#notifications,#notification-panel,#value-popover"))enablePointerGlow(surface);
const parameterToolbar=document.querySelector(".parameter-toolbar");
function anchorToolbar(){
  if(parameterToolbar.classList.contains("is-anchored"))return;
  const groups=[...parameterToolbar.children],rect=parameterToolbar.getBoundingClientRect();
  parameterToolbar.style.setProperty("--anchor-inset",Math.max(0,groups[0].getBoundingClientRect().left-rect.left)+"px");
  for(const group of groups.slice(0,2))group.style.setProperty("--anchor-width",group.getBoundingClientRect().width+"px");
  parameterToolbar.classList.add("is-anchored");
}
function releaseToolbar(){parameterToolbar.classList.remove("is-anchored");parameterToolbar.style.removeProperty("--anchor-inset");for(const group of parameterToolbar.children)group.style.removeProperty("--anchor-width");}
parameterToolbar.addEventListener("pointerenter",anchorToolbar);
parameterToolbar.addEventListener("pointerleave",releaseToolbar);
window.addEventListener("blur",releaseToolbar);
window.addEventListener("resize",()=>{releaseToolbar();if(parameterToolbar.matches(":hover"))anchorToolbar();});
function refreshControls() {
  const method=$("quantization").value,mono=Number($("colors").value)===1,vc=$("tracing").value==="vecline",block=busy||!invoke;
  const active={veclineIterations:method==="vc"&&!mono,despeckle:!vc,smoothCorners:!vc,optimizePaths:!vc&&Number($("smoothCorners").value)>0,dither:!(mono&&method==="vc")};
  for(const def of algorithmParameters){active[def.id]=def.scope==="wl"?method==="wl":def.scope==="monoVc"?mono&&method==="vc":def.scope==="monoFixed"?mono&&method==="vc"&&$("monoThreshold").value==="fixed":def.scope==="monoAdaptive"?mono&&method==="vc"&&$("monoThreshold").value==="adaptive":vc;}
  const curves=vc&&["trace","auto"].includes($("vcMode").value);
  for(const key of ["vcPreset","vcTolerance","vcFitError","vcMinArea","vcOptimizeError","vcGradients"])active[key]&&=curves;
  active.vcGradients&&=!mono;
  for(const button of $("algorithm-scroll").querySelectorAll(".parameter-button"))button.hidden=button.dataset.field.startsWith("vc")?!vc:vc;
  for(const [key,enabled] of Object.entries(active))if($(key))$(key).disabled=block||!enabled;
  if(editingValue&&$(editingValue).disabled)$("value-popover").hidden=true;

  const logo=$("background-logo"),enabled=$("transparency").value==="white";
  logo.setAttribute("aria-pressed",String(enabled));logo.setAttribute("aria-label",`Chromink · 描摹结果白底预览${enabled?"开启":"关闭"}`);
  logo.dataset.tooltip="Chromink · 描彩\n参数：悬停滚轮调节，点击输入或切换，双击恢复默认值。\n预览：滚轮中心缩放，左键拖动，双击切换适应窗口和原图大小。\nLogo：点击或滚轮切换结果白底，按住拖动窗口。\n版本：0.9.6\n主页：https://github.com/skystary/chromink";
  for (const button of document.querySelectorAll(".parameter-button")) {
    const id=button.dataset.field, input=$(id), isNumber=Object.hasOwn(steps,id);
    button.classList.toggle("has-number",isNumber);button.disabled=input.disabled||button.hidden;
    let state="",index=0;
    if (isNumber) {button.innerHTML=`<span class="parameter-icon">${icons[id]}</span><span class="parameter-number">${input.value}</span>`;state=input.value;button.setAttribute("role","spinbutton");button.setAttribute("aria-valuenow",state);button.setAttribute("aria-valuemin",input.min);button.setAttribute("aria-valuemax",input.max);}
    else if (input.tagName==="SELECT") {index=input.selectedIndex;state=input.options[index].textContent;button.innerHTML=icons[id][index];}
    else {button.innerHTML=input.type==="checkbox"&&!input.checked?offIcon:icons[id];if(input.type==="checkbox"){state=input.checked?"开":"关";button.classList.toggle("is-off",!input.checked);button.setAttribute("aria-pressed",String(input.checked));}}
    button.dataset.state=String(index);button.setAttribute("aria-label",`${names[id]}${state?`：${state}`:""}`);
    button.dataset.tooltip=`${names[id]}${state?`：${state}`:""}\n${hints[id]}`;
    button.removeAttribute("title"); if(tooltipOwner===button)paintTooltip(button);
  }
}
function bumpValue(id,direction) {
  const input=$(id);if(input.disabled)return;
  const value=Number(input.value);let next=value+direction*steps[id];
  if(id==="smoothCorners"||id==="optimizePaths"){const step=steps[id];next=(direction>0?Math.floor(value/step+1e-9)+1:Math.ceil(value/step-1e-9)-1)*step;}
  if(id==="prescale"){const step=value<1||value===1&&direction<0 ? .1 : .5;next=value+direction*step;if(value<1&&next>1||value>1&&next<1)next=1;}input.value=Number(Math.max(Number(input.min),Math.min(Number(input.max),next)).toFixed(3));
  input.dispatchEvent(new Event("change"));
}
let editingValue=null;
for (const button of document.querySelectorAll(".parameter-button")) {
  const id=button.dataset.field,input=$(id);
  button.addEventListener("dblclick",event=>{event.preventDefault();if(button.disabled)return;$("value-popover").hidden=true;hideTooltip();if(input.type==="checkbox")input.checked=defaultOptions[id];else input.value=input.type==="number"?Math.min(Number(input.max),defaultOptions[id]):defaultOptions[id];input.dispatchEvent(new Event("change"));});
  if(input.tagName==="SELECT" || input.type==="checkbox")button.addEventListener("wheel",event=>{if(event.shiftKey||Math.abs(event.deltaX)>Math.abs(event.deltaY))return;event.preventDefault();event.stopPropagation();if(button.disabled || !event.deltaY)return;if(input.tagName==="SELECT")input.selectedIndex=(input.selectedIndex+(event.deltaY<0?-1:1)+input.options.length)%input.options.length;else input.checked=!input.checked;input.dispatchEvent(new Event("change"));},{passive:false});
  button.addEventListener("click",()=>{
    if(Object.hasOwn(steps,id)) {editingValue=id;$("value-name").textContent=names[id];$("value-input").value=input.value;$("value-input").min=input.min;$("value-input").max=input.max;$("value-input").step=input.step;$("value-popover").hidden=false;positionValuePopover(button);$("value-input").focus();$("value-input").select();}
    else if(input.tagName==="SELECT") {input.selectedIndex=(input.selectedIndex+1)%input.options.length;input.dispatchEvent(new Event("change"));}else if(input.type==="checkbox"){input.checked=!input.checked;input.dispatchEvent(new Event("change"));}
  });
  if(Object.hasOwn(steps,id)) {
    button.addEventListener("wheel",event=>{if(event.shiftKey||Math.abs(event.deltaX)>Math.abs(event.deltaY))return;event.preventDefault();event.stopPropagation();if(event.deltaY)bumpValue(id,event.deltaY<0?1:-1);},{passive:false});
    button.addEventListener("keydown",event=>{if(["ArrowUp","ArrowDown"].includes(event.key)){event.preventDefault();bumpValue(id,event.key==="ArrowUp"?1:-1);}});
  }
}
function applyValue() {if(!editingValue||!$("value-input").value||!$("value-input").checkValidity()){$("value-input").reportValidity();return;}const input=$(editingValue);if(input.disabled){$("value-popover").hidden=true;return;}input.value=$("value-input").value;$("value-popover").hidden=true;input.dispatchEvent(new Event("change"));$(`${editingValue}-button`).focus();}
$("value-apply").addEventListener("click",applyValue);
$("value-input").addEventListener("keydown",event=>{if(event.key==="Enter"){event.preventDefault();applyValue();}if(event.key==="Escape")$("value-popover").hidden=true;});
for (const canvas of document.querySelectorAll(".canvas")) {
  canvas.removeAttribute("title"); canvas.addEventListener("dblclick", toggleFit);
  canvas.addEventListener("pointerdown", event => { if(event.button!==0 || !current())return; event.preventDefault(); dragging={canvas,x:event.clientX,y:event.clientY};canvas.setPointerCapture?.(event.pointerId);canvas.classList.add("dragging"); });
  canvas.addEventListener("pointermove", event => { if(dragging?.canvas!==canvas)return;const pan=pans.get(canvas.id)||{x:0,y:0};pan.x+=event.clientX-dragging.x;pan.y+=event.clientY-dragging.y;dragging.x=event.clientX;dragging.y=event.clientY;pans.set(canvas.id,pan);zoom(); });
  const end=()=>{dragging=null;canvas.classList.remove("dragging");};canvas.addEventListener("pointerup",end);canvas.addEventListener("lostpointercapture",end);canvas.addEventListener("pointercancel",end);
}
document.addEventListener("contextmenu",event=>{event.preventDefault();if(event.target.closest("#save"))saveCurrent(true);},{capture:true});
window.addEventListener("beforeunload",()=>{clearTimeout(toastTimer);for(const state of panelStates.values())clearTimeout(state.timer);if(traceTimer!==null)clearTimeout(traceTimer);discardPreview();});
if (!invoke) status("请通过桌面应用打开；在项目目录运行 npm run dev。", true);
render();
async function restoreSession(){
  if(!invoke)return;busy=true;updateButtons();
  try{const result=await invoke("restore_session");if(!result)return;
    for(const image of result.images){const key=sourceKey(image);fileOptions.set(key,{...defaultOptions,...image.options});images.push(image);}
    library.nodes=result.library||[];library.expanded=new Set(result.expanded||[]);library.normalize(images);syncLibrary();queueChanged();selected=result.selected??images[0]?.id??null;const image=images.find(i=>i.id===selected);if(image)loadFileOptions(image);fullPreview=null;discardPreview();if(image)invalidate();
    if(result.errors?.length)status(result.errors.join("；"),true);
  }catch(error){status(String(error),true);}
  finally{busy=false;render();if(tracePending&&traceTimer===null)runTrace();}
}
restoreSession();

for(const scroll of [$("algorithm-scroll"),$("reduction-scroll")])scroll.addEventListener("wheel",event=>{if(event.shiftKey||Math.abs(event.deltaX)>Math.abs(event.deltaY)||!event.target.closest(".parameter-button")){event.preventDefault();scroll.scrollLeft+=event.deltaX||event.deltaY;hideTooltip();$("value-popover").hidden=true;}},{passive:false});
