// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

interface SceneView { setActive(active:boolean):void;dispose():void; }
interface ViewOptions {
  select:HTMLSelectElement;
  geometry:readonly HTMLElement[];
  scene:HTMLElement;
  sceneControls:HTMLElement;
  setGeometryActive(active:boolean):void;
  loadScene():Promise<{mountScene(options:{container:HTMLElement;controls:HTMLElement}):Promise<SceneView>}>;
}
/** App-only lazy views: one retained Stage, with rendering paused while hidden. */
export function mountViews(options:ViewOptions):{dispose():void} {
  let disposed=false;
  let sceneState:{kind:"unloaded"}|{kind:"loading"}|{kind:"ready";view:SceneView}={kind:"unloaded"};
  const sceneSelected=()=>options.select.value==="scene";
  function visibility(){
    const scene=sceneSelected();
    for(const element of options.geometry)element.hidden=scene;
    options.scene.hidden=!scene;options.sceneControls.hidden=!scene;
    options.setGeometryActive(!scene);
    if(sceneState.kind==="ready")sceneState.view.setActive(scene);
  }
  function reconcile(){
    if(disposed)return;
    visibility();
    if(!sceneSelected()||sceneState.kind!=="unloaded")return;
    options.scene.textContent="Loading Scene…";
    sceneState={kind:"loading"};
    void(async()=>{
      try {
        const module=await options.loadScene();
        if(disposed)return;
        const candidate=await module.mountScene({container:options.scene,controls:options.sceneControls});
        if(disposed){candidate.dispose();return;}
        sceneState={kind:"ready",view:candidate};
        candidate.setActive(sceneSelected());
      }catch(error){
        if(!disposed){sceneState={kind:"unloaded"};options.scene.textContent=error instanceof Error?error.message:String(error);}
      }
    })();
  }
  options.select.addEventListener("change",reconcile);
  reconcile();
  return {dispose(){
    if(disposed)return;
    disposed=true;
    options.select.removeEventListener("change",reconcile);
    options.setGeometryActive(false);
    if(sceneState.kind==="ready")sceneState.view.dispose();
    sceneState={kind:"unloaded"};
  }};
}
