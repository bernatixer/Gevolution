import { createCube } from '../components/cube'
import { BufferGeometryUtils } from 'three/examples/jsm/utils/BufferGeometryUtils.js';
import { Noise } from 'noisejs';

export const createTerrain = (scene) => {
    var size = 100;
    var terrainGrid = [];
    for (var i=0; i<size; i++) terrainGrid[i] = new Array(size);
    var noise = new Noise(Math.random());

    for (var i=0; i<terrainGrid.length; ++i) {
        for (var j=0; j<terrainGrid[i].length; ++j) {
            var value = noise.perlin2(i/50, j/50);
            terrainGrid[i][j] = Math.abs(value) * 20;
            var cube = createCube(i,0,j,terrainGrid[i][j]<1.5);
            scene.add(cube);
        }
    }
}
