import * as THREE from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader';

export default class ChickenEntity {
    constructor(terrain, scene, physics) {
        this.ready = false;

        let self = this;
        const gltfLoader = new GLTFLoader();
        gltfLoader.load('/assets/chicken.gltf', (gltf) => {
          const root = gltf.scene;
          let height = 0;
          root.traverse(function (child) {
            if (child instanceof THREE.Mesh) {
              child.geometry.center();
              child.geometry.computeBoundingBox();
              const maxY = child.geometry.boundingBox.max.y;
              const minY = child.geometry.boundingBox.min.y;
              height = (maxY-minY)*5;
              child.castShadow = true;
              child.receiveShadow = true;
            }
          });
          root.scale.set(10,10,10);
          let noise = terrain.simplex.noise2D(32/150, 32/150);
          noise = Math.floor(noise*terrain.cellSize/2 + terrain.cellSize/2);
          root.position.x = 32;
          root.position.y = noise+1+height + 20;
          root.position.z = 32;
          root.rotation.y = -Math.PI / 2;
          root.userData.velocity = new THREE.Vector3();
          root.userData.direction = new THREE.Vector3();

          self.object = root;
          scene.add(root);
          physics.add.existing(root, { mass: 2, collisionFlags: 0 })
          
          root.body.setAngularFactor(0,0,0);
          root.body.setFriction(0.05)
          self.ready = true
          // polloHelper = new THREE.BoxHelper(pollo);
          // scene.add(polloHelper);
          root.body.on.collision(function(otherObject, event) {
            if (otherObject.userData.isGround && event == "end") {
                self.isColliding = false;
            } else {
                self.isColliding = true;
            }
          })
        });
    }

}
