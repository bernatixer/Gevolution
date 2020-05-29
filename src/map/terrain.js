import * as THREE from 'three';
import SimplexNoise from 'simplex-noise';

const cellSize = 32;
const mapWidth = 5*cellSize;

export const generateTerrain = (world) => {
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
}

export const drawTerrain = (scene, physics, world) => {
    for (let x = 0; x < mapWidth/cellSize; ++x) {
        for (let z = 0; z < mapWidth/cellSize; ++z) {
          drawCell(scene, physics, world, x,0,z);
        }
    }
}

function drawCell(scene, physics, world, x, y, z) {
    const { positions, normals, colors, indices } = world.generateGeometryDataForCell(x, y, z);
    const geometry = new THREE.BufferGeometry();
    const material = new THREE.MeshLambertMaterial({ vertexColors: true });
  
    const positionNumComponents = 3;
    const normalNumComponents = 3;
    geometry.setAttribute('position', new THREE.BufferAttribute(new Float32Array(positions), positionNumComponents));
    geometry.setAttribute('normal', new THREE.BufferAttribute(new Float32Array(normals), normalNumComponents));
    geometry.setAttribute('color', new THREE.BufferAttribute(new Float32Array(colors), 3));
    geometry.setIndex(indices);
    
    const mesh = new THREE.Mesh(geometry, material);
    mesh.receiveShadow = true;
    mesh.castShadow = true;

    scene.add(mesh);
    physics.add.existing(mesh, { shape: 'concaveMesh' });
    mesh.body.setCollisionFlags(2);
}
