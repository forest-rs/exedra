// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import * as THREE from "three";

/** A sibling display overlay; the caller retains geometry and surface materials. */
export function meshTriangles(surface:THREE.Mesh<THREE.BufferGeometry,THREE.MeshStandardMaterial[]>,visible:boolean) {
  const mesh=new THREE.Mesh(surface.geometry,new THREE.MeshBasicMaterial({color:0x183039,wireframe:true,transparent:true,opacity:.72,depthTest:true,depthWrite:false}));
  mesh.renderOrder=1;
  const setVisible=(show:boolean)=>{
    mesh.visible=show;
    for(const material of surface.material){material.polygonOffset=show;material.polygonOffsetFactor=1;material.polygonOffsetUnits=1;}
  };
  setVisible(visible);
  return {mesh,setVisible,dispose(){mesh.material.dispose();}};
}
