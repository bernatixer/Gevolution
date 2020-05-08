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
import cameras from './components/cameras'

var camera, scene, controls;
var webglRenderer;

var container;
var stats;
var clock = new THREE.Clock();

var pollo;

init();
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
    
    cameras.addAmbientLight(scene);
    cameras.addDirectionalLight(scene);
    
    const cellSize = 32;
    const mapWidth = 3*cellSize;
    camera = createCamera(mapWidth);
    const world = new VoxelWorld(cellSize);
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
      root.position.y = noise+1+height;
      root.position.z = 32;
      scene.add(root);
      pollo = root;
      // var boxHelper = new THREE.BoxHelper(root);
      // scene.add(boxHelper);
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
  scene.add(mesh);
}

function onWindowResize() {
    camera.aspect = window.innerWidth / window.innerHeight;
    camera.updateProjectionMatrix();
    webglRenderer.setSize(window.innerWidth, window.innerHeight);
}

function animate() {
    requestAnimationFrame(animate);

    let delta = clock.getDelta();
    // if (pollo) pollo.translateZ( -1 * delta )

    controls.update();
    render();
}

function render() {
    stats.update();
    webglRenderer.render(scene, camera);
}
