use std::collections::{HashMap, HashSet, VecDeque};

use crate::domain::asset::Asset;
use crate::domain::job::Job;

/// One asset in the lineage graph.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LineageNode {
    pub key: String,
    /// Longest-path depth from root assets (0 = no upstream materializers).
    pub layer: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_materialized_at: Option<String>,
    /// True when an upstream input is fresher than this asset, or when the
    /// asset has never been materialized. This is the "blast radius" signal.
    pub stale: bool,
}

/// A dependency edge: `from` feeds `to`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LineageEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LineageGraph {
    pub nodes: Vec<LineageNode>,
    pub edges: Vec<LineageEdge>,
}

/// Derives the asset dependency graph from pipeline definitions and computes
/// staleness. Pure function over domain data — trivially testable.
///
/// Edges come from tasks that both consume (`asset_inputs`) and produce
/// (`materializes`) assets: input → output. Assets referenced by no pipeline
/// still appear as isolated nodes (from the asset catalog).
pub fn compute_lineage(jobs: &[Job], assets: &[Asset]) -> LineageGraph {
    // Collect every referenced asset and the edges between them.
    let mut referenced: HashSet<String> = HashSet::new();
    let mut edges: Vec<LineageEdge> = Vec::new();
    let mut seen_edges: HashSet<(String, String)> = HashSet::new();

    for job in jobs {
        for task in &job.tasks {
            if let Some(out) = &task.materializes {
                referenced.insert(out.clone());
                for input in &task.asset_inputs {
                    referenced.insert(input.clone());
                    let edge = (input.clone(), out.clone());
                    if input != out && seen_edges.insert(edge.clone()) {
                        edges.push(LineageEdge { from: edge.0, to: edge.1 });
                    }
                }
            }
            for input in &task.asset_inputs {
                referenced.insert(input.clone());
            }
        }
    }
    for asset in assets {
        referenced.insert(asset.key.clone());
    }

    // Freshness index for staleness computation.
    let last_mat: HashMap<&str, Option<chrono::DateTime<chrono::Utc>>> = assets
        .iter()
        .map(|a| (a.key.as_str(), a.last_materialized_at))
        .collect();

    // Topological layering (Kahn). Nodes left after the sweep are part of a
    // cross-pipeline cycle; they get the last layer so they still render.
    let mut indegree: HashMap<&str, usize> = referenced.iter().map(|k| (k.as_str(), 0)).collect();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &edges {
        indegree.entry(e.to.as_str()).and_modify(|d| *d += 1);
        dependents.entry(e.from.as_str()).or_default().push(e.to.as_str());
    }

    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&k, _)| k)
        .collect();
    let mut layer: HashMap<&str, usize> = HashMap::new();
    let mut processed: HashSet<&str> = HashSet::new();
    let mut max_layer = 0usize;
    while let Some(key) = queue.pop_front() {
        processed.insert(key);
        let l = *layer.get(key).unwrap_or(&0);
        max_layer = max_layer.max(l);
        for &child in dependents.get(key).unwrap_or(&vec![]) {
            layer.insert(child, layer.get(child).copied().unwrap_or(0).max(l + 1));
            max_layer = max_layer.max(l + 1);
            if let Some(d) = indegree.get_mut(child) {
                *d -= 1;
                if *d == 0 {
                    queue.push_back(child);
                }
            }
        }
    }
    let cycle_layer = max_layer + 1;

    let mut nodes: Vec<LineageNode> = referenced
        .iter()
        .map(|key| {
            // Roots (never a child) sit at layer 0; cycle members land after
            // every acyclic layer so they still render distinctly.
            let l = if processed.contains(key.as_str()) {
                *layer.get(key.as_str()).unwrap_or(&0)
            } else {
                cycle_layer
            };
            let own = last_mat.get(key.as_str()).copied().flatten();
            let stale = compute_stale(key.as_str(), own, &edges, &last_mat);
            LineageNode {
                key: key.clone(),
                layer: l,
                last_materialized_at: own
                    .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
                stale,
            }
        })
        .collect();
    nodes.sort_by(|a, b| (a.layer, &a.key).cmp(&(b.layer, &b.key)));

    LineageGraph { nodes, edges }
}

/// An asset is stale when it has never been materialized, or when any
/// upstream input was materialized more recently than the asset itself.
fn compute_stale(
    key: &str,
    own_last: Option<chrono::DateTime<chrono::Utc>>,
    edges: &[LineageEdge],
    last_mat: &HashMap<&str, Option<chrono::DateTime<chrono::Utc>>>,
) -> bool {
    let Some(own) = own_last else {
        return true; // never materialized
    };
    edges
        .iter()
        .filter(|e| e.to == key)
        .any(|e| {
            last_mat
                .get(e.from.as_str())
                .copied()
                .flatten()
                .map(|up| up > own)
                .unwrap_or(false)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::job::Task;
    use uuid::Uuid;
    fn task(id: &str, inputs: &[&str], materializes: Option<&str>) -> Task {
        Task {
            id: id.into(),
            name: id.into(),
            command: "true".into(),
            depends_on: vec![],
            asset_inputs: inputs.iter().map(|s| s.to_string()).collect(),
            materializes: materializes.map(|s| s.into()),
            timeout_secs: 60,
            max_retries: 0,
            retry_delay_secs: 1,
        }
    }

    fn job(tasks: Vec<Task>) -> Job {
        Job::new("j", "", tasks, Uuid::new_v4())
    }

    fn asset(key: &str, at: Option<chrono::DateTime<chrono::Utc>>) -> Asset {
        let mut a = Asset::new(key);
        a.last_materialized_at = at;
        a
    }

    #[test]
    fn lineage_edges_and_layers() {
        // raw → staging → warehouse
        let jobs = vec![job(vec![
            task("t1", &["raw"], Some("staging")),
            task("t2", &["staging"], Some("warehouse")),
        ])];
        let g = compute_lineage(&jobs, &[]);
        assert_eq!(g.edges.len(), 2);
        assert!(g.edges.contains(&LineageEdge {
            from: "raw".into(),
            to: "staging".into(),
        }));
        let layer = |k: &str| g.nodes.iter().find(|n| n.key == k).unwrap().layer;
        assert_eq!(layer("raw"), 0);
        assert_eq!(layer("staging"), 1);
        assert_eq!(layer("warehouse"), 2);
    }

    #[test]
    fn staleness_propagates_downstream() {
        let t = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 9, 12, 12, 0, 0).unwrap();
        let jobs = vec![job(vec![task("t", &["raw"], Some("curated"))])];
        // Upstream fresher than downstream → downstream stale.
        let assets = vec![
            asset("raw", Some(t)),
            asset("curated", Some(t - chrono::Duration::hours(1))),
        ];
        let g = compute_lineage(&jobs, &assets);
        let node = |k: &str| g.nodes.iter().find(|n| n.key == k).unwrap().clone();
        assert!(!node("raw").stale);
        assert!(node("curated").stale);

        // Downstream fresh after upstream → not stale.
        let assets = vec![
            asset("raw", Some(t - chrono::Duration::hours(1))),
            asset("curated", Some(t)),
        ];
        let g = compute_lineage(&jobs, &assets);
        assert!(!g.nodes.iter().find(|n| n.key == "curated").unwrap().stale);
    }

    #[test]
    fn never_materialized_is_stale() {
        let jobs = vec![job(vec![task("t", &[], Some("a.b"))])];
        let g = compute_lineage(&jobs, &[]);
        assert!(g.nodes.iter().find(|n| n.key == "a.b").unwrap().stale);
    }

    #[test]
    fn self_reference_ignored() {
        let jobs = vec![job(vec![task("t", &["a"], Some("a"))])];
        let g = compute_lineage(&jobs, &[]);
        assert!(g.edges.is_empty()); // self-edge suppressed
    }

    #[test]
    fn cross_pipeline_cycle_does_not_hang() {
        let jobs = vec![
            job(vec![task("t1", &["b"], Some("a"))]),
            job(vec![task("t2", &["a"], Some("b"))]),
        ];
        let g = compute_lineage(&jobs, &[]); // must terminate
        assert_eq!(g.nodes.len(), 2);
    }
}
