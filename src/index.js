import * as Stats from 'stats-js';
import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls';
import { createCamera } from './components/camera';
import { createScene } from './components/scene';
import { createWebGLRenderer } from './components/webGLRenderer';
import { OBJLoader } from 'three/examples/jsm/loaders/OBJLoader.js';
import { MTLLoader } from 'three/examples/jsm/loaders/MTLLoader.js';
import { AmmoPhysics, PhysicsLoader } from '@enable3d/ammo-physics';
import lights from './components/lights';
import Terrain from './map/terrain';
import EntitiesManager from './entities/entitiesManager';
import { config } from './config';

var camera, scene, controls;
var webglRenderer;

var container;
var stats;
var clock = new THREE.Clock();

var entitiesManager;
let physics;

PhysicsLoader('/assets/lib', () => {
  init()
  animate();
})

function init() {
    container = document.createElement('div');
    document.body.appendChild(container);

    scene = createScene();
    physics = new AmmoPhysics(scene, { gravity: { x: 0, y: -20, z: 0 }});
    // physics.debug.enable(true)

    stats = new Stats();
    container.appendChild(stats.dom);
    stats.dom.style.position = 'absolute';

    // scene.add(new THREE.AxesHelper(4));
    // scene.add(new THREE.GridHelper(32*2, 32*2));
    
    lights.addAmbientLight(scene);
    lights.addDirectionalLight(scene);
    
    const terrain = new Terrain();

    camera = createCamera(terrain.mapWidth);

    terrain.generateTerrain();
    terrain.drawTerrain(scene, physics);
    
    var mtlLoader = new MTLLoader();
    mtlLoader.load('/assets/pollito.obj.mtl', function( materials ) {
    
        materials.preload();
        var objLoader = new OBJLoader();
        objLoader.setMaterials( materials );
        objLoader.load('/assets/pollito.obj', (object) => {
          
          let height = 0;
          object.traverse( function ( child ) {
            if ( child instanceof THREE.Mesh ) {
              child.geometry.center();
              child.geometry.computeBoundingBox();
              const maxY = child.geometry.boundingBox.max.y;
              const minY = child.geometry.boundingBox.min.y;
              height = (maxY-minY)*5;
              child.castShadow = true;
              child.receiveShadow = true;
            }
          } );
          object.rotation.x += -Math.PI / 2;
          object.scale.set(0.1,0.1,0.1);
          object.position.x = 40;
          object.position.y = 35;
          object.position.z = 40;
          object.userData.velocity = new THREE.Vector3();
          object.userData.direction = new THREE.Vector3();
          // entities.push(object);
          scene.add(object);
          physics.add.existing(object, { mass: 4, collisionFlags: 0 })
          object.body.setAngularFactor(0,0,0);
          },
        );
    });

    entitiesManager = new EntitiesManager(terrain, scene, physics);
    entitiesManager.addChicken();

    webglRenderer = createWebGLRenderer();
    container.appendChild(webglRenderer.domElement);
    window.addEventListener('resize', onWindowResize, false);

    controls = new OrbitControls(camera, webglRenderer.domElement);
    controls.enableKeys = false;
    controls.update();
}

function onWindowResize() {
    camera.aspect = config.SCREEN_WIDTH / config.SCREEN_HEIGHT;
    camera.updateProjectionMatrix();
    webglRenderer.setSize(config.SCREEN_WIDTH, config.SCREEN_HEIGHT);
}

function animate() {
  entitiesManager.applyAction();
  physics.update(clock.getDelta() * 1000);
  // physics.updateDebugger();

  requestAnimationFrame(animate);
  controls.update();
  render();
}

var onKeyDown = function ( event ) {
  switch ( event.keyCode ) {
    
    case 37: // left
      entitiesManager.entities[0].object.rotateY(Math.PI / 32);
      break;

    case 39: // right
      entitiesManager.entities[0].object.rotateY(-Math.PI / 32);
      break;

    case 87: // w
      entitiesManager.entities[0].object.body.applyForceX(5);
      break;

    /*case 32: // space
      if ( canJump === true && selectedEntity > -1 && selectedEntity < entities.length) {
        entities[selectedEntity].userData.velocity.y += 20;
      }
      canJump = false;
      entities[1].body.applyForceY(20);
      break;*/

  }
};

document.addEventListener('keydown', onKeyDown, false);

function render() {
    stats.update();
    webglRenderer.render(scene, camera);
}
