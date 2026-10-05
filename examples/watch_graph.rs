//! Query compiled Watch graphs without evaluating them.
use anyhow::{ensure, Result};
use clap::Parser;
use renderer::{formats, watch::WatchData};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
};

#[derive(Parser)]
struct Options {
    engine: PathBuf,
    #[arg(long)]
    node: Option<usize>,
    #[arg(long)]
    operation: Vec<String>,
    #[arg(long)]
    root: Option<usize>,
    #[arg(long)]
    slot: Option<usize>,
    #[arg(long)]
    literal: Option<f64>,
    #[arg(long)]
    block: Option<f64>,
    #[arg(long, default_value_t = 2)]
    depth: usize,
    #[arg(long, default_value_t = 500)]
    limit: usize,
    /// Follow consumers rather than child dependencies.
    #[arg(long)]
    consumers: bool,
    #[arg(long)]
    output: Option<PathBuf>,
    /// Save exact loaded WatchData for offline graph queries.
    #[arg(long)]
    dump: Option<PathBuf>,
}
fn children(w: &WatchData, id: usize) -> Vec<usize> {
    w.nodes[id]
        .args
        .iter()
        .filter_map(Value::as_u64)
        .map(|v| v as usize)
        .collect()
}
fn closure(w: &WatchData, root: usize) -> BTreeSet<usize> {
    let mut found = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if id < w.nodes.len() && found.insert(id) {
            pending.extend(children(w, id));
        }
    }
    found
}
fn main() -> Result<()> {
    let opt = Options::parse();
    let w = formats::load_engine(&opt.engine)?.watch;
    ensure!(opt.limit > 0, "limit must be positive");
    ensure!(
        opt.node.is_none_or(|id| id < w.nodes.len()),
        "node is out of range"
    );
    ensure!(
        opt.root.is_none_or(|id| id < w.nodes.len()),
        "root is out of range"
    );
    ensure!(
        opt.slot.is_none() || opt.root.is_some(),
        "slot requires root (the JumpLoop node)"
    );
    if let (Some(root), Some(slot)) = (opt.root, opt.slot) {
        ensure!(
            w.nodes[root].func.as_deref() == Some("JumpLoop"),
            "slot root must be JumpLoop"
        );
        ensure!(slot < w.nodes[root].args.len(), "slot is out of range");
    }
    if let Some(path) = opt.dump.as_ref() {
        std::fs::write(path, serde_json::to_vec(&w)?)?;
    }
    let mut parents = vec![Vec::new(); w.nodes.len()];
    for id in 0..w.nodes.len() {
        for child in children(&w, id) {
            if child < parents.len() {
                parents[child].push(id);
            }
        }
    }
    let scope = opt.root.map(|r| closure(&w, r));
    let mut seeds = BTreeSet::new();
    if let Some(id) = opt.node {
        seeds.insert(id);
    }
    for (id, n) in w.nodes.iter().enumerate() {
        if (!opt.operation.is_empty() && n.func.as_ref().is_some_and(|v| opt.operation.contains(v)))
            || opt
                .literal
                .is_some_and(|v| n.value.as_ref().and_then(Value::as_f64) == Some(v))
            || opt.block.is_some_and(|v| {
                n.func
                    .as_ref()
                    .is_some_and(|f| f.starts_with("Get") || f.starts_with("Set"))
                    && n.args
                        .first()
                        .and_then(Value::as_u64)
                        .and_then(|i| w.nodes.get(i as usize))
                        .and_then(|n| n.value.as_ref())
                        .and_then(Value::as_f64)
                        == Some(v)
            })
        {
            if scope.as_ref().is_none_or(|s| s.contains(&id)) {
                seeds.insert(id);
            }
        }
    }
    if let (Some(root), Some(slot)) = (opt.root, opt.slot) {
        seeds.extend(
            w.nodes[root]
                .args
                .get(slot)
                .and_then(Value::as_u64)
                .map(|v| v as usize),
        );
    }
    let mut found = BTreeSet::new();
    let mut queue = seeds.iter().map(|&id| (id, 0)).collect::<VecDeque<_>>();
    while found.len() < opt.limit && !queue.is_empty() {
        let (id, d) = queue.pop_front().unwrap();
        if id >= w.nodes.len() || !found.insert(id) {
            continue;
        }
        if d < opt.depth {
            for next in if opt.consumers {
                parents[id].clone()
            } else {
                children(&w, id)
            } {
                queue.push_back((next, d + 1));
            }
        }
    }
    let mut callbacks = BTreeMap::new();
    for a in &w.archetypes {
        for (stage, value) in [
            ("preprocess", &a.preprocess),
            ("spawnTime", &a.spawn_time),
            ("despawnTime", &a.despawn_time),
            ("initialize", &a.initialize),
            ("updateSequential", &a.update_sequential),
            ("updateParallel", &a.update_parallel),
            ("terminate", &a.terminate),
        ] {
            if let Some(root) = value.as_ref().and_then(|v| {
                v.as_u64()
                    .or_else(|| v.get("index").and_then(Value::as_u64))
            }) {
                let reachable = closure(&w, root as usize);
                let hits = seeds.intersection(&reachable).copied().collect::<Vec<_>>();
                if !hits.is_empty() {
                    callbacks.insert(
                        format!("{}.{}", a.name, stage),
                        json!({"root":root,"targets":hits}),
                    );
                }
            }
        }
    }
    let mut loops = Vec::new();
    for (id, _) in w
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.func.as_deref() == Some("JumpLoop"))
    {
        if scope.as_ref().is_some_and(|s| !s.contains(&id)) {
            continue;
        }
        for (slot, child) in children(&w, id).into_iter().enumerate() {
            let reachable = closure(&w, child);
            let hits = seeds.intersection(&reachable).copied().collect::<Vec<_>>();
            if !hits.is_empty() {
                loops.push(json!({"node":id,"slot":slot,"child":child,"targets":hits}));
            }
        }
    }
    let out = json!({"seeds":seeds,"truncated":!queue.is_empty(),"nodes":found.iter().map(|&id|json!({"id":id,"node":w.nodes[id],"consumers":parents[id]})).collect::<Vec<_>>(),"callbacks":callbacks,"loops":loops});
    let bytes = serde_json::to_vec_pretty(&out)?;
    if let Some(path) = opt.output {
        std::fs::write(path, bytes)?;
    } else {
        println!("{}", String::from_utf8(bytes)?);
    }
    Ok(())
}
