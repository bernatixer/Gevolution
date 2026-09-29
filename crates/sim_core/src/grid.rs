//! Regular square grid with canonical edges.
//!
//! Cells are row-major: `c = z * width + x`. Horizontal edges come first,
//! `e = z * (width - 1) + x` joins (x, z) and (x + 1, z); vertical edges follow,
//! `e = H + z * width + x` joins (x, z) and (x, z + 1). Endpoint A is always the
//! lower cell index. Each physical edge has exactly one id.

#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub id: String,
    pub width: usize,
    pub height: usize,
    /// Meters, identical on both axes.
    pub cell_size: f64,
    pub chunk_size: usize,
}

impl Grid {
    pub fn cells(&self) -> usize {
        self.width * self.height
    }
    pub fn horizontal_edges(&self) -> usize {
        self.width.saturating_sub(1) * self.height
    }
    pub fn edges(&self) -> usize {
        self.horizontal_edges() + self.width * self.height.saturating_sub(1)
    }
    pub fn cell_area(&self) -> f64 {
        self.cell_size * self.cell_size
    }
    #[inline]
    pub fn edge_cells(&self, e: usize) -> (usize, usize) {
        let h = self.horizontal_edges();
        if e < h {
            let z = e / (self.width - 1);
            let x = e % (self.width - 1);
            let a = z * self.width + x;
            (a, a + 1)
        } else {
            let a = e - h;
            (a, a + self.width)
        }
    }
    /// Incident edges of a cell in ascending edge-id order: left, right, up, down.
    /// Each entry is (edge id, this cell is endpoint A).
    #[inline]
    pub fn incident(&self, c: usize) -> [Option<(usize, bool)>; 4] {
        let (x, z) = (c % self.width, c / self.width);
        let w1 = self.width - 1;
        let h = self.horizontal_edges();
        [
            (x > 0).then(|| (z * w1 + x - 1, false)),
            (x + 1 < self.width).then(|| (z * w1 + x, true)),
            (z > 0).then(|| (h + (z - 1) * self.width + x, false)),
            (z + 1 < self.height).then(|| (h + z * self.width + x, true)),
        ]
    }
    #[inline]
    pub fn is_boundary(&self, c: usize) -> bool {
        let (x, z) = (c % self.width, c / self.width);
        x == 0 || z == 0 || x + 1 == self.width || z + 1 == self.height
    }
    /// Cell containing a world position, clamped into the grid.
    #[inline]
    pub fn cell_at(&self, x: f64, z: f64) -> usize {
        let cx = ((x / self.cell_size).floor().max(0.0) as usize).min(self.width - 1);
        let cz = ((z / self.cell_size).floor().max(0.0) as usize).min(self.height - 1);
        cz * self.width + cx
    }
    pub fn extent_x(&self) -> f64 {
        self.width as f64 * self.cell_size
    }
    pub fn extent_z(&self) -> f64 {
        self.height as f64 * self.cell_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_edges_are_unique_and_incident() {
        let g = Grid {
            id: "t".into(),
            width: 4,
            height: 3,
            cell_size: 1.0,
            chunk_size: 2,
        };
        let mut count = vec![0; g.cells()];
        for e in 0..g.edges() {
            let (a, b) = g.edge_cells(e);
            assert!(a < b);
            count[a] += 1;
            count[b] += 1;
        }
        for c in 0..g.cells() {
            let inc: Vec<_> = g.incident(c).iter().flatten().copied().collect();
            assert_eq!(inc.len(), count[c]);
            for w in inc.windows(2) {
                assert!(w[0].0 < w[1].0);
            }
            for (e, is_a) in inc {
                let (a, b) = g.edge_cells(e);
                assert_eq!(if is_a { a } else { b }, c);
            }
        }
    }
}
