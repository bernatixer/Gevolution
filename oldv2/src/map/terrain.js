import * as THREE from 'three';
import SimplexNoise from 'simplex-noise';
import VoxelEngine from './voxelEngine';

export default class Terrain {
    constructor() {
        this.cellSize = 32;
        this.mapWidth = 5*this.cellSize;
        this.engine = new VoxelEngine(this.cellSize);
        this.simplex = new SimplexNoise('seed');
    }

    generateTerrain() {
      for (let y = 0; y < this.cellSize; ++y) {
          for (let z = 0; z < this.mapWidth; ++z) {
            for (let x = 0; x < this.mapWidth; ++x) {
              const height = this.simplex.noise2D(x/150, z/150);
              height = Math.floor(height*this.cellSize/2 + this.cellSize/2);
              if (height < 7) height = 7;
              if (y <= height) {
                let color = 1;
                if (height <= 7) {
                  color = 2;
                }
                this.engine.setVoxel(x, y, z, color);
              }
            }
          }
        }
    }

    drawTerrain(scene, physics) {
        for (let x = 0; x < this.mapWidth/this.cellSize; ++x) {
            for (let z = 0; z < this.mapWidth/this.cellSize; ++z) {
              this.drawCell(scene, physics, x,0,z);
            }
        }
    }

    drawCell(scene, physics, x, y, z) {
        const { positions, normals, colors, indices } = this.engine.generateGeometryDataForCell(x, y, z);
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
        mesh.userData.isGround = true;
    }
}
