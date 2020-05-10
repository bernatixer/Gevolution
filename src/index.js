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

      pollo.userData.velocity = new THREE.Vector3(0,0,-5);
      pollo.userData.acceleration = new THREE.Vector3(0,-1,0);
      pollo.userData.lastElapsed = clock.elapsedTime;
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
  const elapsed = clock.getElapsedTime();
  if (pollo) {
    if (isInGround(pollo)) {
      pollo.userData.lastElapsed = elapsed;
    } else {
      pollo.userData.acceleration = new THREE.Vector3(0,-1,0);
    }
    const velocity = pollo.userData.velocity.clone();
    velocity.multiplyScalar(delta);
    const elapsedTime = elapsed - pollo.userData.lastElapsed;
    const acceleration = pollo.userData.acceleration.clone();
    acceleration.multiplyScalar(0.981*((elapsedTime)**2)/2);
    const position = new THREE.Vector3();
    position.addVectors(velocity,acceleration);
    pollo.translateOnAxis(position, 1);
    if (!canMove(pollo)) {
      pollo.userData.lastElapsed = elapsed;
      pollo.translateOnAxis(position, -1);
    }
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
    [ Math.floor(box.min.x), Math.floor(box.min.y-0.15), Math.floor(box.min.z) ],
    [ Math.floor(box.min.x), Math.floor(box.min.y-0.15), Math.floor(box.max.z) ],
    [ Math.floor(box.max.x), Math.floor(box.min.y-0.15), Math.floor(box.min.z) ],
    [ Math.floor(box.max.x), Math.floor(box.min.y-0.15), Math.floor(box.max.z) ],
  ];
}

function render() {
    stats.update();
    webglRenderer.render(scene, camera);
}
