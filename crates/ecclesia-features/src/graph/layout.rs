//! Barnes-Hut O(N log N) force-directed layout step solver for large knowledge graphs (plan F.14).

use super::GraphNode;

/// Parameters controlling force-directed layout physics (plan F.14).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutConfig {
    /// Repulsive Coulomb constant (positive; applied outward between nodes).
    pub repulsion: f64,
    /// Ideal rest length for connected springs.
    pub spring_length: f64,
    /// Hooke spring stiffness along edges.
    pub spring_stiffness: f64,
    /// Central gravity pulling disconnected components toward `(0, 0)`.
    pub gravity: f64,
    /// Velocity damping in `0.0..1.0`.
    pub damping: f64,
    /// Barnes-Hut opening angle threshold (`0.5..=1.0`, default `0.75`).
    pub theta: f64,
    /// Maximum displacement per integration step (stabilizes high-degree hubs).
    pub max_displacement: f64,
    /// Time step multiplier.
    pub dt: f64,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            repulsion: 1800.0,
            spring_length: 90.0,
            spring_stiffness: 0.06,
            gravity: 0.02,
            damping: 0.85,
            theta: 0.75,
            max_displacement: 32.0,
            dt: 1.0,
        }
    }
}

/// Summary statistics from a single layout solver step.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutMetrics {
    /// Sum of squared node speeds (`0.5 * mass * v^2`).
    pub kinetic_energy: f64,
    /// Maximum position delta applied to any node during the step.
    pub max_displacement: f64,
}

#[derive(Debug, Clone)]
struct QuadNode {
    cx: f64,
    cy: f64,
    half_size: f64,
    total_mass: f64,
    com_x: f64,
    com_y: f64,
    body: Option<(usize, f64, f64, f64)>,
    children: Option<[usize; 4]>,
}

struct QuadTree {
    nodes: Vec<QuadNode>,
}

impl QuadTree {
    fn build(points: &[(f64, f64, f64)]) -> Self {
        if points.is_empty() {
            return Self { nodes: Vec::new() };
        }
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for &(x, y, _) in points {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        let span = (max_x - min_x).max(max_y - min_y).max(1.0);
        let root = QuadNode {
            cx: (min_x + max_x) * 0.5,
            cy: (min_y + max_y) * 0.5,
            half_size: span * 0.5 + 1.0,
            total_mass: 0.0,
            com_x: 0.0,
            com_y: 0.0,
            body: None,
            children: None,
        };
        let mut tree = Self {
            nodes: Vec::with_capacity(points.len().saturating_mul(2)),
        };
        tree.nodes.push(root);
        for (idx, &(x, y, mass)) in points.iter().enumerate() {
            tree.insert(0, idx, x, y, mass, 0);
        }
        tree
    }

    fn insert(&mut self, node_idx: usize, body_idx: usize, x: f64, y: f64, mass: f64, depth: u8) {
        let m = mass.max(0.1);
        let prev_mass = self.nodes[node_idx].total_mass;
        let new_mass = prev_mass + m;
        self.nodes[node_idx].com_x = (self.nodes[node_idx].com_x * prev_mass + x * m) / new_mass;
        self.nodes[node_idx].com_y = (self.nodes[node_idx].com_y * prev_mass + y * m) / new_mass;
        self.nodes[node_idx].total_mass = new_mass;

        if self.nodes[node_idx].children.is_none() && self.nodes[node_idx].body.is_none() {
            self.nodes[node_idx].body = Some((body_idx, x, y, m));
            return;
        }
        // Cap recursion depth for coincident or nearly coincident coordinates.
        if depth >= 24 {
            return;
        }

        if let Some((ex_id, ex_x, ex_y, ex_m)) = self.nodes[node_idx].body.take() {
            self.subdivide(node_idx);
            let q_old = self.quadrant(node_idx, ex_x, ex_y);
            let child_old = self.nodes[node_idx].children.expect("subdivided")[q_old];
            self.insert(child_old, ex_id, ex_x, ex_y, ex_m, depth + 1);
        }

        if self.nodes[node_idx].children.is_none() {
            self.subdivide(node_idx);
        }
        let q = self.quadrant(node_idx, x, y);
        let child = self.nodes[node_idx].children.expect("subdivided")[q];
        self.insert(child, body_idx, x, y, m, depth + 1);
    }

    fn subdivide(&mut self, node_idx: usize) {
        let parent = &self.nodes[node_idx];
        let half = parent.half_size * 0.5;
        let cx = parent.cx;
        let cy = parent.cy;
        let offsets = [
            (cx - half, cy - half),
            (cx + half, cy - half),
            (cx - half, cy + half),
            (cx + half, cy + half),
        ];
        let base = self.nodes.len();
        for (ox, oy) in offsets {
            self.nodes.push(QuadNode {
                cx: ox,
                cy: oy,
                half_size: half,
                total_mass: 0.0,
                com_x: 0.0,
                com_y: 0.0,
                body: None,
                children: None,
            });
        }
        self.nodes[node_idx].children = Some([base, base + 1, base + 2, base + 3]);
    }

    fn quadrant(&self, node_idx: usize, x: f64, y: f64) -> usize {
        let n = &self.nodes[node_idx];
        let right = usize::from(x >= n.cx);
        let bottom = usize::from(y >= n.cy);
        bottom * 2 + right
    }

    fn repulsion_force(
        &self,
        body_idx: usize,
        x: f64,
        y: f64,
        theta: f64,
        repulsion: f64,
    ) -> (f64, f64) {
        if self.nodes.is_empty() {
            return (0.0, 0.0);
        }
        let mut fx = 0.0;
        let mut fy = 0.0;
        let mut stack = Vec::with_capacity(64);
        stack.push(0_usize);

        while let Some(idx) = stack.pop() {
            let node = &self.nodes[idx];
            if node.total_mass <= 0.0 || node.body.is_some_and(|(id, ..)| id == body_idx) {
                continue;
            }
            let dx = x - node.com_x;
            let dy = y - node.com_y;
            let dist_sq = dx * dx + dy * dy + 1.0;
            let dist = dist_sq.sqrt();
            let width = node.half_size * 2.0;

            if node.children.is_none() || (width / dist) < theta {
                let mag = (repulsion * node.total_mass) / dist_sq;
                fx += (dx / dist) * mag;
                fy += (dy / dist) * mag;
            } else if let Some(children) = node.children {
                for c in children {
                    if self.nodes[c].total_mass > 0.0 {
                        stack.push(c);
                    }
                }
            }
        }
        (fx, fy)
    }
}

/// Runs one Barnes-Hut force-directed layout iteration over `nodes` and indexed `edges`.
pub(crate) fn step_barnes_hut(
    nodes: &mut [GraphNode],
    edges: &[(usize, usize, f64)],
    config: &LayoutConfig,
) -> LayoutMetrics {
    if nodes.is_empty() {
        return LayoutMetrics::default();
    }

    let points: Vec<(f64, f64, f64)> = nodes.iter().map(|n| (n.x, n.y, n.mass)).collect();
    let tree = QuadTree::build(&points);

    let mut forces = vec![(0.0_f64, 0.0_f64); nodes.len()];
    for (idx, node) in nodes.iter().enumerate() {
        let (rx, ry) = tree.repulsion_force(idx, node.x, node.y, config.theta, config.repulsion);
        forces[idx].0 += rx - node.x * config.gravity;
        forces[idx].1 += ry - node.y * config.gravity;
    }

    for &(src, dst, weight) in edges {
        if src >= nodes.len() || dst >= nodes.len() || src == dst {
            continue;
        }
        let dx = nodes[dst].x - nodes[src].x;
        let dy = nodes[dst].y - nodes[src].y;
        let dist = (dx * dx + dy * dy).sqrt().max(0.001);
        let disp = (dist - config.spring_length) * config.spring_stiffness * weight.max(0.1);
        let fx = (dx / dist) * disp;
        let fy = (dy / dist) * disp;
        forces[src].0 += fx;
        forces[src].1 += fy;
        forces[dst].0 -= fx;
        forces[dst].1 -= fy;
    }

    let mut metrics = LayoutMetrics::default();
    for (idx, node) in nodes.iter_mut().enumerate() {
        if node.pinned {
            node.vx = 0.0;
            node.vy = 0.0;
            continue;
        }
        let inv_mass = 1.0 / node.mass.max(0.1);
        node.vx = (node.vx + forces[idx].0 * inv_mass * config.dt) * config.damping;
        node.vy = (node.vy + forces[idx].1 * inv_mass * config.dt) * config.damping;

        let mut dx = node.vx * config.dt;
        let mut dy = node.vy * config.dt;
        let step_len = (dx * dx + dy * dy).sqrt();
        if step_len > config.max_displacement && step_len > 0.0 {
            let scale = config.max_displacement / step_len;
            dx *= scale;
            dy *= scale;
        }
        node.x += dx;
        node.y += dy;
        let applied = (dx * dx + dy * dy).sqrt();
        metrics.max_displacement = metrics.max_displacement.max(applied);
        metrics.kinetic_energy += 0.5 * node.mass * (node.vx * node.vx + node.vy * node.vy);
    }

    metrics
}
