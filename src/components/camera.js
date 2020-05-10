import * as THREE from 'three';

export const createCamera = (mapWidth) => {
    var camera = new THREE.PerspectiveCamera(75, window.innerWidth / window.innerHeight, 1, 5000);
    camera.position.set(mapWidth/2, mapWidth/4, mapWidth/2);
    camera.up = new THREE.Vector3(0,1,0);
    camera.lookAt(mapWidth/2, 0, mapWidth/2);
    return camera;
}
