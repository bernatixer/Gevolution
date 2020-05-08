import * as THREE from 'three';

const funcs = {
    addDirectionalLight(scene) {
        var directionalLight = new THREE.DirectionalLight(0xffffff, 1);
        directionalLight.position.set(100,100,100);
        directionalLight.castShadow = true;
    
        scene.add(directionalLight);
        const helper = new THREE.DirectionalLightHelper(directionalLight);
        scene.add(helper);
        
        const d = 320;
        // directionalLight.shadow.bias = -0.0001;
        directionalLight.shadow.camera.left = -d;
        directionalLight.shadow.camera.right = d;
        directionalLight.shadow.camera.top = d;
        directionalLight.shadow.camera.bottom = -d;
        
        // const shadowHelper = new THREE.CameraHelper( directionalLight.shadow.camera );
        // scene.add(shadowHelper);
    }, 
    addAmbientLight(scene) {
        const ambientLight = new THREE.AmbientLight(0x666666);
        scene.add(ambientLight);
    },
}

export default funcs