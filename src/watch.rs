use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchData {
    #[serde(default)] pub archetypes: Vec<WatchArchetype>,
    #[serde(default)] pub nodes: Vec<EngineNode>,
    #[serde(default)] pub skin: Value,
    #[serde(default)] pub effect: Value,
    #[serde(default)] pub particle: Value,
    #[serde(default)] pub update_spawn: Option<Value>,
    #[serde(default)] pub buckets: Vec<Value>,
    #[serde(flatten)] pub extra: BTreeMap<String, Value>,
}
/// A compiled Sonolus node. `func` is deliberately an open string: engines may
/// use any operation name, even when this renderer has no VM implementation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineNode {
    #[serde(default)] pub func: Option<String>,
    #[serde(default)] pub args: Vec<Value>,
    #[serde(default)] pub value: Option<Value>,
    #[serde(flatten)] pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchArchetype {
    #[serde(default)] pub name: String,
    #[serde(default)] pub imports: Vec<Value>,
    #[serde(default)] pub exports: Vec<Value>,
    #[serde(flatten)] pub callbacks: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WatchInventory {
    pub archetypes: Vec<String>,
    pub functions: Vec<String>,
    pub memory_blocks: Vec<String>,
    pub skin_bindings: Value,
    pub effect_bindings: Value,
    pub particle_bindings: Value,
}

pub fn inventory(data: &WatchData) -> WatchInventory {
    let mut functions = Vec::new(); let mut memory = Vec::new();
    fn walk(v: &Value, functions: &mut Vec<String>, memory: &mut Vec<String>) {
        match v {
            Value::Object(o) => {
                for key in ["name", "function", "operator", "type"] { if let Some(s)=o.get(key).and_then(Value::as_str) { if ["function","operator"].contains(&key) || (key=="type" && o.contains_key("args")) || (key=="name" && (o.contains_key("args") || o.contains_key("arguments") || o.contains_key("inputs"))) { functions.push(s.to_string()); } } }
                for (k,val) in o { if k.to_ascii_lowercase().contains("memory") || k.to_ascii_lowercase().contains("block") { if let Some(s)=val.as_str(){memory.push(s.into())} else if val.is_number(){memory.push(val.to_string())} } walk(val,functions,memory); }
            }, Value::Array(a) => for x in a {walk(x,functions,memory)}, _=>{}
        }
    }
    for node in &data.nodes {
        if let Some(func) = &node.func {
            functions.push(func.clone());
            if ["Get", "Set", "GetPointed", "SetPointed", "GetShifted", "SetShifted"].contains(&func.as_str()) {
                if let Some(block_arg) = node.args.first().and_then(Value::as_u64).and_then(|i| data.nodes.get(i as usize)) {
                    if let Some(block_id) = block_arg.value.as_ref() { memory.push(block_id.to_string()); }
                }
            }
        }
        walk(&Value::Object(node.extra.iter().map(|(k,v)|(k.clone(),v.clone())).chain(node.value.iter().map(|v|(String::from("value"),v.clone()))).collect()), &mut functions, &mut memory);
    }
    walk(&serde_json::to_value(&data.archetypes).unwrap_or(Value::Null), &mut functions, &mut memory);
    functions.sort(); functions.dedup(); memory.sort(); memory.dedup();
    WatchInventory { archetypes:data.archetypes.iter().map(|a|a.name.clone()).collect(), functions, memory_blocks:memory, skin_bindings:data.skin.clone(), effect_bindings:data.effect.clone(), particle_bindings:data.particle.clone() }
}
