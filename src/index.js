import * as Stats from 'stats-js';
import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls'
import { createCamera } from './components/camera'
import { createScene } from './components/scene'
import { createWebGLRenderer } from './components/webGLRenderer'
import VoxelWorld from './map/voxelWorld'
import * as TERRAIN from './map/terrain'
import {GLTFLoader} from 'three/examples/jsm/loaders/GLTFLoader';
import {OBJLoader} from 'three/examples/jsm/loaders/OBJLoader.js';
import {MTLLoader} from 'three/examples/jsm/loaders/MTLLoader.js';
import lights from './components/lights'
import {AmmoPhysics, PhysicsLoader} from '@enable3d/ammo-physics';
import * as NEAT from 'neataptic';
import SimplexNoise from 'simplex-noise';

var camera, scene, controls;
var webglRenderer;

var container;
var stats;
var clock = new THREE.Clock();

var world;
var polloHelper;
var entities = [];
var selectedEntity = 1;

var moveForward = false;
var moveBackward = false;
var moveLeft = false;
var moveRight = false;
var canJump = false;

var isColliding = false;

let physics;

PhysicsLoader('/assets/lib', () => {
  init()
  animate();
})

var network = new NEAT.architect.Perceptron(2, 2, 1);

var trainingSet = [
  { input: [0,-1], output: [0] },
  { input: [0,-1.5], output: [0] },
  { input: [0,-2.5], output: [0] },
  { input: [0,0], output: [0] },
  { input: [0,0.1], output: [0] },
  { input: [0,0.2], output: [0] },
  { input: [0,0.3], output: [0] },
  { input: [0,0.4], output: [0] },
  { input: [0,0.5], output: [0] },
  { input: [0,1.1], output: [0] },
  { input: [0,1.2], output: [0] },
  { input: [0,1.3], output: [0] },
  { input: [0,1.4], output: [0] },
  { input: [0,1.5], output: [0] },
  { input: [1,0], output: [1] },
  { input: [1,0.1], output: [1] },
  { input: [1,0.11], output: [1] },

  { input: [0,1], output: [0] },
  { input: [1,1], output: [0] },

  { input: [0,2], output: [0] },

  { input: [0,3], output: [0] },
  { input: [0,4], output: [0] },
  { input: [0,5], output: [0] },
];
network.train(trainingSet, {
  error: 0.01
});

// console.log(network.activate([0,0.2467564]))

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
    
    const cellSize = 32;
    const mapWidth = 5*cellSize;
    camera = createCamera(mapWidth);

    
    world = new VoxelWorld(cellSize);
    TERRAIN.generateTerrain(world);
    TERRAIN.drawTerrain(scene, physics, world);
    
    const simplex = new SimplexNoise('seed');
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
          entities.push(object);
          scene.add(object);
          physics.add.existing(object, { mass: 4, collisionFlags: 0 })
          object.body.setAngularFactor(0,0,0);
          },
        );
    });

    const gltfLoader = new GLTFLoader();
    gltfLoader.load('/assets/model.gltf', (gltf) => {
      const root = gltf.scene;
      let height = 0;
      root.traverse(function ( child ) {
        if ( child instanceof THREE.Mesh ) {
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
      let noise = simplex.noise2D(32/150, 32/150);
      noise = Math.floor(noise*cellSize/2 + cellSize/2);
      root.position.x = 32;
      root.position.y = noise+1+height + 20;
      root.position.z = 32;
      root.rotation.y = -Math.PI / 2;
      root.userData.velocity = new THREE.Vector3();
      root.userData.direction = new THREE.Vector3();
      entities.push(root);
      scene.add(root);
      physics.add.existing(root, { mass: 2, collisionFlags: 0 })
      root.body.setAngularFactor(0,0,0);
      root.body.setFriction(0.05)
      // polloHelper = new THREE.BoxHelper(pollo);
      // scene.add(polloHelper);
      root.body.on.collision( function(otherObject, event) {
        if (otherObject.name !== 'ground') {
          isColliding = true;
        }
      })
    });

    webglRenderer = createWebGLRenderer();
    container.appendChild(webglRenderer.domElement);
    window.addEventListener('resize', onWindowResize, false);

    controls = new OrbitControls( camera, webglRenderer.domElement );
    controls.enableKeys = false;
    controls.update();
}

function onWindowResize() {
    camera.aspect = window.innerWidth / window.innerHeight;
    camera.updateProjectionMatrix();
    webglRenderer.setSize(window.innerWidth, window.innerHeight);
}

function animate() {
  for (let i=0; i<entities.length; ++i) {
    jump(entities[i])
  }
  isColliding = false;
  physics.update(clock.getDelta() * 1000);
  // physics.updateDebugger();
  requestAnimationFrame(animate);
  /*const delta = clock.getDelta();
  for (let i=0; i<entities.length; ++i) {
    moveEntity(entities[i], delta);
  }*/
  controls.update();
  render();
}

function jump(entity) {
  const velocity = entity.body.velocity.y;
  const jump = network.activate([isColliding ? 1 : 0, velocity])[0];
  if (jump > 0.90) entity.body.applyForceY(20);
}

function moveEntity(entity, delta) {
  if (entity) {
    entity.userData.velocity.x -= entity.userData.velocity.x * 20.0 * delta;
    entity.userData.velocity.z -= entity.userData.velocity.z * 20.0 * delta;

    // entity.userData.direction.z = Number( moveForward ) - Number( moveBackward );
    // entity.userData.direction.x = Number( moveRight ) - Number( moveLeft );
    // entity.userData.direction.normalize();

    // if ( moveForward || moveBackward ) entity.userData.velocity.z -= entity.userData.direction.z * 400.0 * delta;
    // if ( moveLeft || moveRight ) entity.userData.velocity.x -= entity.userData.direction.x * 400.0 * delta;
    if ( moveForward ) entity.userData.velocity.z -= 400.0 * delta;

    if (!isInGround(entity)) {
      entity.userData.velocity.y -= 9.81 * 5 * delta; // 10 = mass
      canJump = false;
    } else {
      canJump = true;
    }

    entity.translateX(-entity.userData.velocity.x * delta);
    entity.translateZ(entity.userData.velocity.z * delta);
    entity.translateY(entity.userData.velocity.y * delta);
    /*let translated = new THREE.Vector3(0,0,0);
    entity.getWorldDirection(translated);
    let defaultGravity;
    if (translated.y) {
      defaultGravity = translated.add(new THREE.Vector3(0,-1,1));
    } else {
      defaultGravity = new THREE.Vector3(0,1,0);
    }
    entity.translateOnAxis(defaultGravity, entity.userData.velocity.y * delta)*/
    
    if (!canMove(entity)) {
      // Intentem moure en eix Z-Y
      entity.translateX(entity.userData.velocity.x * delta);
      entity.userData.velocity.x = 0;

      // Intentem moure en eix Y
      if (!canMove(entity)) {
        entity.translateZ(-entity.userData.velocity.z * delta);
        entity.userData.velocity.z = 0;

        // No es pot moure
        if (!canMove(entity)) {
          entity.translateY(-entity.userData.velocity.y * delta);
          entity.userData.velocity.y = Math.max( 0, entity.userData.velocity.y );
        }
      }
    }

    // polloHelper.update();
  }
}

function canMove(entity) {
  let canMove = true;
  const vertices = getBoxVertices(entity);
  vertices.forEach(vertex => {
    if (world.getVoxel(...vertex)) {
      canMove = false;
      return;
    }
  });
  return canMove;
}

function isInGround(entity) {
  let inGround = false;
  const vertices = getLowerVertices(entity);
  vertices.forEach(vertex => {
    if (world.getVoxel(...vertex)) {
      inGround = true;
      return;
    }
  });
  return inGround;
}

function getBoxVertices(object) {
  const box = new THREE.Box3().setFromObject(object);
  return [
    [ Math.floor(box.min.x), Math.floor(box.min.y), Math.floor(box.min.z) ],
    [ Math.floor(box.min.x), Math.floor(box.min.y), Math.floor(box.max.z) ],
    [ Math.floor(box.min.x), Math.floor(box.max.y), Math.floor(box.min.z) ],
    [ Math.floor(box.min.x), Math.floor(box.max.y), Math.floor(box.max.z) ],
    [ Math.floor(box.max.x), Math.floor(box.min.y), Math.floor(box.min.z) ],
    [ Math.floor(box.max.x), Math.floor(box.min.y), Math.floor(box.max.z) ],
    [ Math.floor(box.max.x), Math.floor(box.max.y), Math.floor(box.min.z) ],
    [ Math.floor(box.max.x), Math.floor(box.max.y), Math.floor(box.max.z) ],
  ];
}

function getLowerVertices(object) {
  const box = new THREE.Box3().setFromObject(object);
  return [
    [ Math.floor(box.min.x), Math.floor(box.min.y-0.1), Math.floor(box.min.z) ],
    [ Math.floor(box.min.x), Math.floor(box.min.y-0.1), Math.floor(box.max.z) ],
    [ Math.floor(box.max.x), Math.floor(box.min.y-0.1), Math.floor(box.min.z) ],
    [ Math.floor(box.max.x), Math.floor(box.min.y-0.1), Math.floor(box.max.z) ],
  ];
}

var onKeyDown = function ( event ) {
  switch ( event.keyCode ) {
    
    case 37: // left
      entities[selectedEntity].rotateY(Math.PI / 32);
      break;

    case 39: // right
      entities[selectedEntity].rotateY(-Math.PI / 32);
      break;

    case 87: // w
      moveForward = true;
      entities[1].body.applyForceX(5);
      break;

    case 65: // a
      moveLeft = true;
      break;

    case 83: // s
      moveBackward = true;
      break;

    case 68: // d
      moveRight = true;
      break;

    case 32: // space
      if ( canJump === true && selectedEntity > -1 && selectedEntity < entities.length) {
        entities[selectedEntity].userData.velocity.y += 20;
      }
      canJump = false;
      entities[1].body.applyForceY(20);
      break;

  }
};

var onKeyUp = function ( event ) {
  switch ( event.keyCode ) {
    case 87: // w
      moveForward = false;
      break;

    case 65: // a
      moveLeft = false;
      break;

    case 83: // s
      moveBackward = false;
      break;

    case 68: // d
      moveRight = false;
      break;
  }
};

document.addEventListener( 'keydown', onKeyDown, false );
document.addEventListener( 'keyup', onKeyUp, false );

function render() {
    stats.update();
    webglRenderer.render(scene, camera);
}
