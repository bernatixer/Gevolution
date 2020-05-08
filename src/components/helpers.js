import * as THREE from 'three';

export const createHelpers = () => {
    var axes = new THREE.AxesHelper(4);
    var gridXZ = new THREE.GridHelper(32*2, 32*2);
    return [axes, gridXZ];
}
