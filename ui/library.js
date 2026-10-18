"use strict";
// The tree stores source paths, never transient image IDs or trace settings.
class FileLibrary {
  constructor(nodes=[], expanded=[]) {this.nodes=nodes;this.expanded=new Set(expanded);this.serial=0;}
  static sourceKey(image){return image.sourceKey||image.pdfGroup||`image:${image.id}`;}
  key(node){return node.kind==="source"?`source:${node.key}`:`group:${node.id}`;}
  normalize(images){
    const available=new Set(images.map(FileLibrary.sourceKey)),seen=new Set(),groups=new Set();
    const clean=(nodes,depth=0)=>Array.isArray(nodes)&&depth<24?nodes.flatMap(node=>{
      if(node?.kind==="source"&&available.has(node.key)&&!seen.has(node.key)){seen.add(node.key);return [{kind:"source",key:node.key}];}
      if(node?.kind==="group"&&typeof node.id==="string"&&!groups.has(node.id)){groups.add(node.id);const children=clean(node.children,depth+1);return children.length?[{kind:"group",id:node.id,name:typeof node.name==="string"?node.name:"分组",children}]:[];}
      return [];
    }):[];
    this.nodes=clean(this.nodes);
    for(const key of available)if(!seen.has(key)){this.nodes.push({kind:"source",key});seen.add(key);}
    const order=new Map([...available].map((key,index)=>[key,index]));
    const rank=node=>node.kind==="source"?(order.get(node.key)??Infinity):Math.min(...node.children.map(rank));
    const sort=nodes=>{for(const node of nodes)if(node.kind==="group")sort(node.children);nodes.sort((a,b)=>rank(a)-rank(b));};sort(this.nodes);
    return this;
  }
  find(key,nodes=this.nodes){for(let index=0;index<nodes.length;index++){const node=nodes[index];if(this.key(node)===key)return {node,nodes,index};if(node.kind==="group"){const found=this.find(key,node.children);if(found)return found;}}return null;}
  contains(node,key){return this.key(node)===key||(node.kind==="group"&&node.children.some(child=>this.contains(child,key)));}
  move(sourceKey,targetKey){
    const source=this.find(sourceKey),target=this.find(targetKey);
    if(!source||!target||this.contains(source.node,targetKey))return false;
    const moving=source.node;source.nodes.splice(source.index,1);
    const destination=this.find(targetKey);
    if(destination.node.kind==="group"){destination.node.children.push(moving);this.expanded.delete(targetKey);}
    else {let id;do{id=`g-${Date.now()}-${++this.serial}`;}while(this.find(`group:${id}`));const group={kind:"group",id,name:`分组 ${this.serial}`,children:[destination.node,moving]};destination.nodes.splice(destination.index,1,group);this.expanded.delete(this.key(group));}
    return true;
  }
  ungroup(key){const found=this.find(key);if(!found||found.node.kind!=="group")return false;found.nodes.splice(found.index,1,...found.node.children);this.expanded.delete(key);return true;}
  toggle(key){if(this.expanded.has(key))this.expanded.delete(key);else this.expanded.add(key);}
  reveal(image){const key=`source:${FileLibrary.sourceKey(image)}`;const walk=nodes=>nodes.some(node=>{if(this.key(node)===key){this.expanded.add(key);return true;}if(node.kind==="group"&&walk(node.children)){this.expanded.add(this.key(node));return true;}return false;});walk(this.nodes);}
  flatten(images){
    const sources=new Map();for(const image of images){const key=FileLibrary.sourceKey(image);if(!sources.has(key))sources.set(key,[]);sources.get(key).push(image);}
    const rows=[];
    const previews=node=>node.kind==="source"?(sources.get(node.key)||[]).slice(0,3):node.children.flatMap(previews).slice(0,3);
    const leaves=(members,depth,owner)=>{for(const image of members)rows.push({key:`image:${image.id}`,type:"image",image,label:image.name,depth,nodeKey:owner});};
    const walk=(nodes,depth=0)=>{for(const node of nodes){const key=this.key(node);
      if(node.kind==="group"){rows.push({key,type:"group",label:node.name,depth,nodeKey:key,open:this.expanded.has(key),count:node.children.length,previewImages:previews(node)});if(this.expanded.has(key))walk(node.children,depth+1);continue;}
      const members=sources.get(node.key)||[];if(!members.length)continue;
      const image=members[0];
      if(members.length===1&&!image.pdfGroup&&(image.frame==null||image.frame===0)){rows.push({key,type:"image",image,label:image.sourceName||image.name,depth,nodeKey:key});continue;}
      rows.push({key,type:"source",label:image.sourceName||image.pdfName||image.name,depth,nodeKey:key,open:this.expanded.has(key),count:members.length,image});
      if(!this.expanded.has(key))continue;
      leaves(members,depth+1,key);
    }};walk(this.nodes);return rows;
  }
}
if(typeof module!=="undefined")module.exports=FileLibrary;
if(typeof window!=="undefined")window.FileLibrary=FileLibrary;
