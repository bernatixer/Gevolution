import * as THREE from 'three';

export default class TerrainEngine {
  constructor(cellSize) {
    this.cellSize = cellSize;
    this.cellSliceSize = cellSize * cellSize;
    this.cells = {};
  }

  cube_faces = [
    { // left
      dir: [ -1,  0,  0, ],
      corners: [
        [ 0, 1, 0 ],
        [ 0, 0, 0 ],
        [ 0, 1, 1 ],
        [ 0, 0, 1 ],
      ],
    },
    { // right
      dir: [  1,  0,  0, ],
      corners: [
        [ 1, 1, 1 ],
        [ 1, 0, 1 ],
        [ 1, 1, 0 ],
        [ 1, 0, 0 ],
      ],
    },
    { // bottom
      dir: [  0, -1,  0, ],
      corners: [
        [ 1, 0, 1 ],
        [ 0, 0, 1 ],
        [ 1, 0, 0 ],
        [ 0, 0, 0 ],
      ],
    },
    { // top
      dir: [  0,  1,  0, ],
      corners: [
        [ 0, 1, 1 ],
        [ 1, 1, 1 ],
        [ 0, 1, 0 ],
        [ 1, 1, 0 ],
      ],
    },
    { // back
      dir: [  0,  0, -1, ],
      corners: [
        [ 1, 0, 0 ],
        [ 0, 0, 0 ],
        [ 1, 1, 0 ],
        [ 0, 1, 0 ],
      ],
    },
    { // front
      dir: [  0,  0,  1, ],
      corners: [
        [ 0, 0, 1 ],
        [ 1, 0, 1 ],
        [ 0, 1, 1 ],
        [ 1, 1, 1 ],
      ],
    },
  ];

  computeCellId(x, y, z) {
    const {cellSize} = this;
    const cellX = Math.floor(x / cellSize);
    const cellY = Math.floor(y / cellSize);
    const cellZ = Math.floor(z / cellSize);
    return `${cellX},${cellY},${cellZ}`;
  }

  computeVoxelOffset(x, y, z) {
    const {cellSize, cellSliceSize} = this;
    const voxelX = THREE.MathUtils.euclideanModulo(x, cellSize) | 0;
    const voxelY = THREE.MathUtils.euclideanModulo(y, cellSize) | 0;
    const voxelZ = THREE.MathUtils.euclideanModulo(z, cellSize) | 0;
    return voxelY * cellSliceSize +
           voxelZ * cellSize +
           voxelX;
  }

  getCellForVoxel(x, y, z) {
    return this.cells[this.computeCellId(x, y, z)];
  }

  setVoxel(x, y, z, v) {
    let cell = this.getCellForVoxel(x, y, z);
    if (!cell) {
      cell = this.addCellForVoxel(x, y, z);
    }
    const voxelOffset = this.computeVoxelOffset(x, y, z);
    cell[voxelOffset] = v;
  }

  addCellForVoxel(x, y, z) {
    const cellId = this.computeCellId(x, y, z);
    let cell = this.cells[cellId];
    if (!cell) {
      const {cellSize} = this;
      cell = new Uint8Array(cellSize * cellSize * cellSize);
      this.cells[cellId] = cell;
    }
    return cell;
  }

  getVoxel(x, y, z) {
    const cell = this.getCellForVoxel(x, y, z);
    if (!cell) {
      return 0;
    }
    const voxelOffset = this.computeVoxelOffset(x, y, z);
    return cell[voxelOffset];
  }

  generateGeometryDataForCell(cellX, cellY, cellZ) {
    const {cellSize} = this;
    const positions = [];
    const normals = [];
    const colors = [];
    const indices = [];
    const startX = cellX * cellSize;
    const startY = cellY * cellSize;
    const startZ = cellZ * cellSize;

    for (let y = 0; y < cellSize; ++y) {
      const voxelY = startY + y;
      for (let z = 0; z < cellSize; ++z) {
        const voxelZ = startZ + z;
        for (let x = 0; x < cellSize; ++x) {
          const voxelX = startX + x;
          const voxel = this.getVoxel(voxelX, voxelY, voxelZ);
          if (voxel) {
            // There is a voxel here but do we need faces for it?
            for (const {dir, corners} of this.cube_faces) {
              const neighbor = this.getVoxel(
                  voxelX + dir[0],
                  voxelY + dir[1],
                  voxelZ + dir[2]);
              if (!neighbor) {
                // this voxel has no neighbor in this direction so we need a face.
                const ndx = positions.length / 3;
                for (const pos of corners) {
                  positions.push((pos[0] + x)+(this.cellSize*cellX), (pos[1] + y)+(this.cellSize*cellY), (pos[2] + z)+(this.cellSize*cellZ));
                  normals.push(...dir);
                }
                indices.push(
                  ndx, ndx + 1, ndx + 2,
                  ndx + 2, ndx + 1, ndx + 3,
                );

                const green = [0.251, 0.498, 0.243, 0.251, 0.498, 0.243, 0.251, 0.498, 0.243, 0.251, 0.498, 0.243];
                const blue = [0.2, 0.463, 0.737, 0.2, 0.463, 0.737, 0.2, 0.463, 0.737, 0.2, 0.463, 0.737];
                if (voxel == 1) {
                  colors.push(...green);
                } else {
                  colors.push(...blue);
                }
              }
            }
          }
        }
      }
    }

    return {
      positions,
      normals,
      colors,
      indices,
    };
  }
}