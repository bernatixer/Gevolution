import * as Stats from 'stats-js';
import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls'
import { createCamera } from './components/camera'
import { createScene } from './components/scene'
import { createHelpers } from './components/helpers'
import { createWebGLRenderer } from './components/webGLRenderer'
import VoxelWorld from './components/voxelWorld'
import SimplexNoise from 'simplex-noise';
import {GLTFLoader} from 'three/examples/jsm/loaders/GLTFLoader';
import lights from './components/lights'

var camera, scene, controls;
var webglRenderer;

var container;
var stats;
var clock = new THREE.Clock();

var world;
var pollo, polloHelper;

var moveForward = false;
var moveBackward = false;
var moveLeft = false;
var moveRight = false;
var canJump = false;

init()
animate();

function init() {
    container = document.createElement('div');
    document.body.appendChild(container);

    scene = createScene();

    stats = new Stats();
    container.appendChild(stats.dom);
    stats.dom.style.position = 'absolute';

    // var [axes, gridXZ] = createHelpers()
    // scene.add(axes);
    // scene.add(gridXZ);
    
    lights.addAmbientLight(scene);
    lights.addDirectionalLight(scene);
    
    const cellSize = 32;
    const mapWidth = 5*cellSize;
    camera = createCamera(mapWidth);
    world = new VoxelWorld(cellSize);
    const simplex = new SimplexNoise('seed');

    for (let y = 0; y < cellSize; ++y) {
      for (let z = 0; z < mapWidth; ++z) {
        for (let x = 0; x < mapWidth; ++x) {
          const height = simplex.noise2D(x/150, z/150);
          height = Math.floor(height*cellSize/2 + cellSize/2);
          if (height < 7) height = 7;
          if (y <= height) {
            let color = 1;
            if (height <= 7) {
              color = 2;
            }
            world.setVoxel(x, y, z, color);
          }
        }
      }
    }

    for (let x = 0; x < mapWidth/cellSize; ++x) {
      for (let z = 0; z < mapWidth/cellSize; ++z) {
        drawCell(world, x,0,z);
      }
    }
    
    const gltfLoader = new GLTFLoader();
    const url = '/assets/model.gltf';
    gltfLoader.load(url, (gltf) => {
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
      scene.add(root);
      pollo = root;

      pollo.userData.velocity = new THREE.Vector3();
      pollo.userData.direction = new THREE.Vector3();
      polloHelper = new THREE.BoxHelper(pollo);
      scene.add(polloHelper);
    });

    webglRenderer = createWebGLRenderer();
    container.appendChild(webglRenderer.domElement);
    window.addEventListener('resize', onWindowResize, false);

    controls = new OrbitControls( camera, webglRenderer.domElement );
    controls.update();
}

function drawCell(world, x, y, z) {
  const {positions, normals, colors, indices} = world.generateGeometryDataForCell(x, y, z);
  const geometry = new THREE.BufferGeometry();
  const material = new THREE.MeshLambertMaterial({ vertexColors: true });

  const positionNumComponents = 3;
  const normalNumComponents = 3;
  geometry.setAttribute(
      'position',
      new THREE.BufferAttribute(new Float32Array(positions), positionNumComponents));
  geometry.setAttribute(
      'normal',
      new THREE.BufferAttribute(new Float32Array(normals), normalNumComponents));
  geometry.setAttribute(
      'color',
      new THREE.BufferAttribute(new Float32Array(colors), 3));
  geometry.setIndex(indices);
  const mesh = new THREE.Mesh(geometry, material);
  mesh.receiveShadow = true;
  mesh.castShadow = true;
  scene.add(mesh);
}

function onWindowResize() {
    camera.aspect = window.innerWidth / window.innerHeight;
    camera.updateProjectionMatrix();
    webglRenderer.setSize(window.innerWidth, window.innerHeight);
}

function animate() {
    requestAnimationFrame(animate);
    movePollo();
    controls.update();
    render();
}

function movePollo() {
  const delta = clock.getDelta();

  if (pollo) {
    pollo.userData.velocity.x -= pollo.userData.velocity.x * 20.0 * delta;
    pollo.userData.velocity.z -= pollo.userData.velocity.z * 20.0 * delta;

    pollo.userData.direction.z = Number( moveForward ) - Number( moveBackward );
    pollo.userData.direction.x = Number( moveRight ) - Number( moveLeft );
    pollo.userData.direction.normalize();

    if ( moveForward || moveBackward ) pollo.userData.velocity.z -= pollo.userData.direction.z * 400.0 * delta;
    if ( moveLeft || moveRight ) pollo.userData.velocity.x -= pollo.userData.direction.x * 400.0 * delta;

    if (!isInGround(pollo)) {
      pollo.userData.velocity.y -= 9.8 * 15 * delta; // 15 = mass
      canJump = false;
    } else {
      canJump = true;
    }

    pollo.translateX(-pollo.userData.velocity.x * delta, 1);
    pollo.translateZ(pollo.userData.velocity.z * delta, 1);
    pollo.translateY(pollo.userData.velocity.y * delta, 1);
    
    if (!canMove(pollo)) {
      // Intentem moure en eix Z-Y
      pollo.translateX(pollo.userData.velocity.x * delta, 1);
      pollo.userData.velocity.x = 0;

      // Intentem moure en eix Y
      if (!canMove(pollo)) {
        pollo.translateZ(-pollo.userData.velocity.z * delta, 1);
        pollo.userData.velocity.z = 0;

        // No es pot moure
        if (!canMove(pollo)) {
          pollo.translateY(-pollo.userData.velocity.y * delta, 1);
          pollo.userData.velocity.y = Math.max( 0, pollo.userData.velocity.y );
        }
      }
    }

    // pollo.translateY( pollo.userData.velocity.y * delta, 1 );
    polloHelper.update();
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
    case 38: // up
    case 87: // w
      moveForward = true;
      break;

    case 37: // left
    case 65: // a
      moveLeft = true;
      break;

    case 40: // down
    case 83: // s
      moveBackward = true;
      break;

    case 39: // right
    case 68: // d
      moveRight = true;
      break;

    case 32: // space
      if ( canJump === true ) pollo.userData.velocity.y += 20;
      canJump = false;
      break;

  }
};

var onKeyUp = function ( event ) {
  switch ( event.keyCode ) {
    case 38: // up
    case 87: // w
      moveForward = false;
      break;

    case 37: // left
    case 65: // a
      moveLeft = false;
      break;

    case 40: // down
    case 83: // s
      moveBackward = false;
      break;

    case 39: // right
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
