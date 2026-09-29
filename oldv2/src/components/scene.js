import * as THREE from 'three';

export const createScene = () => {
    var scene = new THREE.Scene();
    scene.background = new THREE.Color('skyblue');
    return scene;
}
