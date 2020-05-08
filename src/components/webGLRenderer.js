import * as THREE from 'three';

export const createWebGLRenderer = () => {
    const canvas = document.querySelector('#c');
    var webglRenderer = new THREE.WebGLRenderer({ canvas, antialias: true });
    webglRenderer.shadowMap.enabled = true;
    webglRenderer.shadowMap.type = THREE.PCFSoftShadowMap;
    webglRenderer.setPixelRatio(window.devicePixelRatio);
    webglRenderer.setSize(window.innerWidth, window.innerHeight);
    return webglRenderer;
}
