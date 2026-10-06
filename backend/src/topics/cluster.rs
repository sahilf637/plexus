//! Grouping items into a tree of topics from a weighted graph of how strongly they relate.
//!
//! Louvain (modularity) finds groups; a higher *resolution* finds smaller ones. The top level
//! is found at a low resolution, and each group that is still big is split again at a higher
//! one, until the groups are small, stop splitting, or the tree is deep enough. So the depth
//! follows the library's size and shape rather than a fixed number of layers.
//!
//! Everything here is deterministic: nodes are visited in index order and ties go to the lower
//! community, so the same graph always gives the same tree.

/// Weighted neighbours of each node (undirected: an edge appears in both lists).
pub type Adjacency = Vec<Vec<(usize, f64)>>;

#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Resolution for the top level: lower finds fewer, bigger groups.
    pub top_resolution: f64,
    /// Resolution when splitting a group further.
    pub child_resolution: f64,
    /// Groups at least this big are split again.
    pub split_min: usize,
    /// Groups smaller than this are folded into their closest sibling.
    pub min_group: usize,
    /// Most levels in the tree (the top level is 1).
    pub max_depth: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self { top_resolution: 0.5, child_resolution: 1.2, split_min: 10, min_group: 3, max_depth: 4 }
    }
}

/// One group in the tree: leaves hold items, the others hold sub-groups.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The items in a leaf; empty for a group that has sub-groups.
    pub items: Vec<usize>,
    pub children: Vec<Group>,
}

impl Group {
    /// Every item in this group and its sub-groups.
    pub fn all_items(&self) -> Vec<usize> {
        let mut found = self.items.clone();
        for child in &self.children {
            found.extend(child.all_items());
        }
        found
    }
}

/// The tree for a graph: the top-level groups, and the items that relate to nothing (`loose`).
pub struct Tree {
    pub groups: Vec<Group>,
    pub loose: Vec<usize>,
}

pub fn build(adj: &Adjacency, params: &Params) -> Tree {
    let n = adj.len();
    let loose: Vec<usize> = (0..n).filter(|&i| adj[i].is_empty()).collect();
    let linked: Vec<usize> = (0..n).filter(|&i| !adj[i].is_empty()).collect();
    if linked.is_empty() {
        return Tree { groups: Vec::new(), loose };
    }
    let (sub, _) = subgraph(adj, &linked);
    let partition = louvain(&sub, params.top_resolution);
    let mut groups = partition_groups(&partition, &linked);
    let leftovers = fold_small(&mut groups, adj, params.min_group);
    let groups = groups.into_iter().map(|members| split(adj, members, params, 1)).collect();
    let mut loose = loose;
    loose.extend(leftovers);
    loose.sort_unstable();
    Tree { groups, loose }
}

/// Splits a group again if it is big enough and falls apart into several sizeable pieces.
fn split(adj: &Adjacency, members: Vec<usize>, params: &Params, depth: usize) -> Group {
    if members.len() < params.split_min || depth >= params.max_depth {
        return Group { items: members, children: Vec::new() };
    }
    let (sub, _) = subgraph(adj, &members);
    let partition = louvain(&sub, params.child_resolution);
    let mut pieces = partition_groups(&partition, &members);
    // A piece too small to stand alone joins its closest sibling (or stays with the parent's
    // biggest piece when nothing links it).
    let strays = fold_small(&mut pieces, adj, params.min_group);
    if pieces.len() < 2 {
        return Group { items: members, children: Vec::new() };
    }
    if !strays.is_empty() {
        let biggest = (0..pieces.len()).max_by_key(|&i| pieces[i].len()).unwrap_or(0);
        pieces[biggest].extend(strays);
    }
    let children: Vec<Group> = pieces.into_iter().map(|piece| split(adj, piece, params, depth + 1)).collect();
    Group { items: Vec::new(), children }
}

/// The groups of a partition, as lists of the original indices (`nodes[i]` is node i's index).
fn partition_groups(partition: &[usize], nodes: &[usize]) -> Vec<Vec<usize>> {
    let count = partition.iter().max().map_or(0, |m| m + 1);
    let mut groups = vec![Vec::new(); count];
    for (i, &community) in partition.iter().enumerate() {
        groups[community].push(nodes[i]);
    }
    groups.retain(|g| !g.is_empty());
    groups
}

/// Folds groups smaller than `min` into the group they are most strongly linked to. Items of a
/// small group that link to no other group are returned.
fn fold_small(groups: &mut Vec<Vec<usize>>, adj: &Adjacency, min: usize) -> Vec<usize> {
    let mut stray = Vec::new();
    loop {
        // Smallest first, so the tiniest pieces are folded before they could absorb others.
        let Some(small) = (0..groups.len()).filter(|&g| groups[g].len() < min).min_by_key(|&g| (groups[g].len(), g)) else {
            return stray;
        };
        if groups.len() == 1 {
            return stray;
        }
        let members = groups.remove(small);
        let mut pull = vec![0.0; groups.len()];
        for &item in &members {
            for &(other, weight) in &adj[item] {
                if let Some(g) = groups.iter().position(|group| group.contains(&other)) {
                    pull[g] += weight;
                }
            }
        }
        let best = (0..groups.len()).filter(|&g| pull[g] > 0.0).max_by(|&a, &b| pull[a].total_cmp(&pull[b]).then(b.cmp(&a)));
        match best {
            Some(g) => groups[g].extend(members),
            None => stray.extend(members),
        }
    }
}

/// The graph restricted to `nodes`, with their indices renumbered 0.. in order.
fn subgraph(adj: &Adjacency, nodes: &[usize]) -> (Adjacency, Vec<usize>) {
    let mut local = vec![usize::MAX; adj.len()];
    for (i, &node) in nodes.iter().enumerate() {
        local[node] = i;
    }
    let sub = nodes
        .iter()
        .map(|&node| adj[node].iter().filter(|(other, _)| local[*other] != usize::MAX).map(|&(other, w)| (local[other], w)).collect())
        .collect();
    (sub, nodes.to_vec())
}

/// Louvain: the community of every node, numbered from 0 in the order communities first appear.
pub fn louvain(adj: &Adjacency, resolution: f64) -> Vec<usize> {
    let n = adj.len();
    if n == 0 {
        return Vec::new();
    }
    // The graph being worked on: starts as the input, then each community becomes one node.
    let mut level_adj: Adjacency = adj.to_vec();
    let mut self_weight = vec![0.0; n];
    // Which community each original node is in, as of the current level.
    let mut of_original: Vec<usize> = (0..n).collect();

    loop {
        let (community, improved) = one_level(&level_adj, &self_weight, resolution);
        if !improved {
            break;
        }
        let count = community.iter().max().map_or(0, |m| m + 1);
        for c in of_original.iter_mut() {
            *c = community[*c];
        }
        // Collapse each community into a single node.
        let mut merged: Vec<std::collections::BTreeMap<usize, f64>> = vec![Default::default(); count];
        let mut merged_self = vec![0.0; count];
        for (node, edges) in level_adj.iter().enumerate() {
            let c = community[node];
            merged_self[c] += self_weight[node];
            for &(other, weight) in edges {
                let d = community[other];
                if c == d {
                    // Seen from both ends, so half each time.
                    merged_self[c] += weight / 2.0;
                } else {
                    *merged[c].entry(d).or_insert(0.0) += weight;
                }
            }
        }
        level_adj = merged.into_iter().map(|m| m.into_iter().collect()).collect();
        self_weight = merged_self;
        if count == 1 {
            break;
        }
    }
    renumber(&of_original)
}

/// Moves nodes one at a time to the neighbouring community that raises modularity most, until
/// nothing moves. Returns each node's community (numbered from 0) and whether anything moved.
fn one_level(adj: &Adjacency, self_weight: &[f64], resolution: f64) -> (Vec<usize>, bool) {
    let n = adj.len();
    let degree: Vec<f64> = (0..n).map(|i| adj[i].iter().map(|(_, w)| w).sum::<f64>() + 2.0 * self_weight[i]).collect();
    let two_m: f64 = degree.iter().sum();
    let mut community: Vec<usize> = (0..n).collect();
    if two_m <= 0.0 {
        return (community, false);
    }
    let mut total = degree.clone(); // The summed degree of each community.
    let mut improved = false;
    // Scratch space: the weight from the current node to each neighbouring community.
    let mut toward = vec![0.0; n];
    for _ in 0..50 {
        let mut moved = false;
        for i in 0..n {
            let mine = community[i];
            let mut touched: Vec<usize> = Vec::new();
            for &(other, weight) in &adj[i] {
                if other == i {
                    continue;
                }
                let c = community[other];
                if toward[c] == 0.0 {
                    touched.push(c);
                }
                toward[c] += weight;
            }
            touched.sort_unstable();
            total[mine] -= degree[i];
            // What staying put is worth, then the best move that beats it.
            let gain = |c: usize, links: f64| links - resolution * total[c] * degree[i] / two_m;
            let mut best = mine;
            let mut best_gain = gain(mine, toward[mine]);
            for &c in &touched {
                if c == mine {
                    continue;
                }
                let g = gain(c, toward[c]);
                if g > best_gain + 1e-12 {
                    best = c;
                    best_gain = g;
                }
            }
            total[best] += degree[i];
            community[i] = best;
            if best != mine {
                moved = true;
                improved = true;
            }
            for c in touched {
                toward[c] = 0.0;
            }
            toward[mine] = 0.0;
        }
        if !moved {
            break;
        }
    }
    (renumber(&community), improved)
}

/// Renumbers communities 0.. in order of first appearance.
fn renumber(community: &[usize]) -> Vec<usize> {
    let mut map = std::collections::HashMap::new();
    community
        .iter()
        .map(|&c| {
            let next = map.len();
            *map.entry(c).or_insert(next)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cliques of `size` nodes, each pair linked with `weight`, plus one weak bridge between
    /// neighbouring cliques.
    pub(crate) fn cliques(count: usize, size: usize, weight: f64, bridge: f64) -> Adjacency {
        let n = count * size;
        let mut adj: Adjacency = vec![Vec::new(); n];
        let link = |a: usize, b: usize, w: f64, adj: &mut Adjacency| {
            adj[a].push((b, w));
            adj[b].push((a, w));
        };
        for c in 0..count {
            for a in 0..size {
                for b in (a + 1)..size {
                    link(c * size + a, c * size + b, weight, &mut adj);
                }
            }
            if c + 1 < count {
                link(c * size, (c + 1) * size, bridge, &mut adj);
            }
        }
        adj
    }

    #[test]
    fn louvain_finds_cliques() {
        let adj = cliques(4, 6, 1.0, 0.1);
        let partition = louvain(&adj, 1.0);
        for c in 0..4 {
            let first = partition[c * 6];
            assert!((0..6).all(|i| partition[c * 6 + i] == first), "clique {c} was split");
        }
        let distinct: std::collections::HashSet<_> = partition.iter().collect();
        assert_eq!(distinct.len(), 4);
    }

    #[test]
    fn a_lower_resolution_gives_bigger_groups() {
        let adj = cliques(6, 5, 1.0, 0.6);
        let coarse = louvain(&adj, 0.1).iter().collect::<std::collections::HashSet<_>>().len();
        let fine = louvain(&adj, 2.0).iter().collect::<std::collections::HashSet<_>>().len();
        assert!(coarse < fine, "{coarse} groups at 0.1, {fine} at 2.0");
    }

    #[test]
    fn the_same_graph_gives_the_same_tree() {
        let adj = cliques(8, 7, 1.0, 0.3);
        let a = build(&adj, &Params::default());
        let b = build(&adj, &Params::default());
        assert_eq!(a.groups, b.groups);
        assert_eq!(a.loose, b.loose);
    }

    #[test]
    fn unlinked_items_are_loose_and_tiny_groups_are_folded_in() {
        let mut adj = cliques(2, 8, 1.0, 0.2);
        // Node 16 links only to node 0 (a group of one), node 17 to nothing.
        adj.push(vec![(0, 0.5)]);
        adj[0].push((16, 0.5));
        adj.push(Vec::new());
        let tree = build(&adj, &Params::default());
        assert_eq!(tree.loose, vec![17]);
        let every: Vec<usize> = tree.groups.iter().flat_map(|g| g.all_items()).collect();
        assert!(every.contains(&16), "the lone linked item joins the group it links to");
        assert!(tree.groups.iter().all(|g| g.all_items().len() >= 3));
    }

    #[test]
    fn depth_follows_size() {
        // Four dense clusters of 30, loosely bridged: one level of ~4 groups, each splitting
        // further only if it has sub-structure.
        let adj = cliques(4, 30, 1.0, 0.05);
        let tree = build(&adj, &Params::default());
        assert!(tree.groups.len() >= 2);
        let total: usize = tree.groups.iter().map(|g| g.all_items().len()).sum();
        assert_eq!(total + tree.loose.len(), 120);
        // No group is split into a single child.
        fn no_only_children(g: &Group) -> bool {
            g.children.len() != 1 && g.children.iter().all(no_only_children)
        }
        assert!(tree.groups.iter().all(no_only_children));
    }
}
