import * as THREE from 'three';

export const createCube = (x,y,z,water) => {
  var boxgeometry = new THREE.BoxBufferGeometry(1, 1, 1);
  var color = 0x8bc34a;
  if (water) color = 0x2196F3;
  var boxmaterial = new THREE.MeshBasicMaterial({ color: color });

  var cube = new THREE.Mesh(boxgeometry, boxmaterial);
  // cube.castShadow = true;
  cube.position.x = x;
  cube.position.y = y + 0.5;
  cube.position.z = z;
  return cube;
}
