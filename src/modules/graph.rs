/// Module dependency graph — topological sort, cycle detection, deletion safety.

use std::collections::{HashMap, HashSet, BTreeSet};
use super::ModuleDescriptor;

#[derive(Debug)]
pub struct ModuleGraph {
    /// All known modules, keyed by name
    pub modules: HashMap<String, ModuleDescriptor>,
    /// Forward edges: module name -> set of modules it depends on
    pub edges: HashMap<String, HashSet<String>>,
    /// Reverse edges: module name -> set of modules that depend on it
    pub reverse_edges: HashMap<String, HashSet<String>>,
}

impl ModuleGraph {
    /// Build from a collection of descriptors. Validates:
    /// - No duplicate module names
    /// - All requires refer to existing modules
    /// - No cycles
    pub fn build(descriptors: Vec<ModuleDescriptor>) -> Result<Self, String> {
        let mut modules = HashMap::new();
        let mut edges: HashMap<String, HashSet<String>> = HashMap::new();
        let mut reverse_edges: HashMap<String, HashSet<String>> = HashMap::new();

        // Check for duplicates and build maps
        for desc in descriptors {
            if modules.contains_key(&desc.name) {
                return Err(format!("duplicate module: {}", desc.name));
            }
            let name = desc.name.clone();
            edges.insert(name.clone(), desc.requires.iter().cloned().collect());
            reverse_edges.entry(name.clone()).or_default();
            modules.insert(name, desc);
        }

        // Validate all requires point to existing modules
        for (name, deps) in edges.iter() {
            for dep in deps.iter() {
                if !modules.contains_key(dep.as_str()) {
                    return Err(format!("module '{}' requires unknown module '{}'", name, dep));
                }
                reverse_edges.entry(dep.clone()).or_default().insert(name.clone());
            }
        }

        let graph = ModuleGraph { modules, edges, reverse_edges };

        // Check for cycles
        graph.check_cycles()?;

        Ok(graph)
    }

    /// Detect cycles via DFS. Reports the cycle path if found.
    fn check_cycles(&self) -> Result<(), String> {
        let mut visited = HashSet::new();
        let mut in_stack = HashSet::new();
        let mut path = Vec::new();

        // Visit in sorted order for determinism
        let mut names: Vec<&String> = self.modules.keys().collect();
        names.sort();

        for name in names {
            if !visited.contains(name.as_str()) {
                self.dfs_cycle(name, &mut visited, &mut in_stack, &mut path)?;
            }
        }
        Ok(())
    }

    fn dfs_cycle(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        in_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> Result<(), String> {
        visited.insert(node.to_string());
        in_stack.insert(node.to_string());
        path.push(node.to_string());

        if let Some(deps) = self.edges.get(node) {
            let mut sorted_deps: Vec<&String> = deps.iter().collect();
            sorted_deps.sort();
            for dep in sorted_deps {
                if !visited.contains(dep.as_str()) {
                    self.dfs_cycle(dep, visited, in_stack, path)?;
                } else if in_stack.contains(dep.as_str()) {
                    // Found a cycle — build the cycle path
                    let cycle_start = path.iter().position(|n| n == dep).unwrap();
                    let cycle: Vec<String> = path[cycle_start..].to_vec();
                    return Err(format!(
                        "dependency cycle: {} -> {}",
                        cycle.join(" -> "),
                        dep
                    ));
                }
            }
        }

        in_stack.remove(node);
        path.pop();
        Ok(())
    }

    /// Topological sort using Kahn's algorithm.
    /// Uses BTreeSet for deterministic tie-breaking (lexicographic).
    pub fn topo_sort(&self) -> Result<Vec<String>, String> {
        // Compute in-degrees: in_degree[A] = number of modules A depends on
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        for (name, deps) in &self.edges {
            in_degree.insert(name.clone(), deps.len());
        }

        // Start with nodes that have no dependencies
        let mut ready: BTreeSet<String> = BTreeSet::new();
        for (name, &deg) in &in_degree {
            if deg == 0 {
                ready.insert(name.clone());
            }
        }

        let mut order = Vec::new();
        while let Some(name) = ready.pop_first() {
            order.push(name.clone());

            // "Remove" this node: decrease in-degree of everything that depends on it
            if let Some(dependents) = self.reverse_edges.get(&name) {
                for dep in dependents {
                    let deg = in_degree.get_mut(dep).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        ready.insert(dep.clone());
                    }
                }
            }
        }

        if order.len() != self.modules.len() {
            return Err("cycle detected during topological sort".into());
        }

        Ok(order)
    }

    /// Check if module `name` can be safely removed.
    /// Returns Err with list of dependents if not.
    pub fn can_remove(&self, name: &str) -> Result<(), Vec<String>> {
        if let Some(dependents) = self.reverse_edges.get(name) {
            if !dependents.is_empty() {
                let mut deps: Vec<String> = dependents.iter().cloned().collect();
                deps.sort();
                return Err(deps);
            }
        }
        Ok(())
    }

    /// Return all transitive dependents of a module (for reload cascading).
    /// Returns in topological order (safe to reload in this order).
    pub fn transitive_dependents(&self, name: &str) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        self.collect_dependents(name, &mut visited, &mut result);
        // Return in topo order by filtering the full topo sort
        if let Ok(full_order) = self.topo_sort() {
            let dep_set: HashSet<&str> = result.iter().map(|s| s.as_str()).collect();
            full_order.into_iter().filter(|n| dep_set.contains(n.as_str())).collect()
        } else {
            result
        }
    }

    fn collect_dependents(&self, name: &str, visited: &mut HashSet<String>, result: &mut Vec<String>) {
        if let Some(dependents) = self.reverse_edges.get(name) {
            for dep in dependents {
                if visited.insert(dep.clone()) {
                    result.push(dep.clone());
                    self.collect_dependents(dep, visited, result);
                }
            }
        }
    }
}

// ── Free functions operating on (name, requires) pairs ──
// These let the ModuleLoader compute dependency info from heap-resident
// ModuleImage objects without building a full ModuleGraph.

/// Topological sort from (name, requires) pairs. Kahn's algorithm.
pub fn topo_sort_pairs(modules: &[(String, Vec<String>)]) -> Result<Vec<String>, String> {
    let names: HashSet<&str> = modules.iter().map(|(n, _)| n.as_str()).collect();

    // Build edges + reverse edges
    let mut edges: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut reverse: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (name, requires) in modules {
        let deps: HashSet<&str> = requires.iter()
            .filter_map(|r| if names.contains(r.as_str()) { Some(r.as_str()) } else { None })
            .collect();
        edges.insert(name.as_str(), deps);
        reverse.entry(name.as_str()).or_default();
    }
    for (name, requires) in modules {
        for req in requires {
            if names.contains(req.as_str()) {
                reverse.entry(req.as_str()).or_default().insert(name.as_str());
            }
        }
    }

    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    for (name, deps) in &edges {
        in_degree.insert(name, deps.len());
    }

    let mut ready: BTreeSet<&str> = BTreeSet::new();
    for (name, &deg) in &in_degree {
        if deg == 0 {
            ready.insert(name);
        }
    }

    let mut order = Vec::new();
    while let Some(name) = ready.pop_first() {
        order.push(name.to_string());
        if let Some(dependents) = reverse.get(name) {
            for dep in dependents {
                let deg = in_degree.get_mut(dep).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    ready.insert(dep);
                }
            }
        }
    }

    if order.len() != modules.len() {
        return Err("cycle detected during topological sort".into());
    }

    Ok(order)
}

/// Transitive dependents from (name, requires) pairs, in topological order.
pub fn transitive_dependents_pairs(modules: &[(String, Vec<String>)], name: &str) -> Vec<String> {
    // Build reverse edges
    let mut reverse: HashMap<&str, Vec<&str>> = HashMap::new();
    for (mod_name, requires) in modules {
        for req in requires {
            reverse.entry(req.as_str()).or_default().push(mod_name.as_str());
        }
    }

    // BFS/DFS to collect all transitive dependents
    let mut visited = HashSet::new();
    let mut stack = vec![name];
    while let Some(n) = stack.pop() {
        if let Some(deps) = reverse.get(n) {
            for dep in deps {
                if visited.insert(*dep) {
                    stack.push(dep);
                }
            }
        }
    }

    // Return in topo order
    if let Ok(full_order) = topo_sort_pairs(modules) {
        full_order.into_iter().filter(|n| visited.contains(n.as_str())).collect()
    } else {
        visited.into_iter().map(|s| s.to_string()).collect()
    }
}

/// Check if a module can be removed (no dependents).
pub fn can_remove_pairs(modules: &[(String, Vec<String>)], name: &str) -> Result<(), Vec<String>> {
    let dependents: Vec<String> = modules.iter()
        .filter(|(_, requires)| requires.iter().any(|r| r == name))
        .map(|(n, _)| n.clone())
        .collect();
    if dependents.is_empty() {
        Ok(())
    } else {
        Err(dependents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn desc(name: &str, requires: &[&str], provides: &[&str]) -> ModuleDescriptor {
        ModuleDescriptor {
            name: name.to_string(),
            requires: requires.iter().map(|s| s.to_string()).collect(),
            provides: provides.iter().map(|s| s.to_string()).collect(),
            path: Some(PathBuf::from(format!("lib/{}.moof", name))),
            source_hash: String::new(),
            body_offset: 0,
            unrestricted: false,
        }
    }

    #[test]
    fn test_topo_sort_basic() {
        let graph = ModuleGraph::build(vec![
            desc("bootstrap", &[], &["Object"]),
            desc("collections", &["bootstrap"], &["Assoc"]),
            desc("classes", &["bootstrap", "collections"], &["Stack"]),
        ]).unwrap();

        let order = graph.topo_sort().unwrap();
        assert_eq!(order, vec!["bootstrap", "collections", "classes"]);
    }

    #[test]
    fn test_topo_sort_correct_order() {
        let graph = ModuleGraph::build(vec![
            desc("bootstrap", &[], &["Object"]),
            desc("collections", &["bootstrap"], &["Assoc"]),
            desc("classes", &["bootstrap", "collections"], &["Stack"]),
        ]).unwrap();

        let order = graph.topo_sort().unwrap();
        // bootstrap first (no deps), then collections (depends on bootstrap),
        // then classes (depends on both)
        let bootstrap_pos = order.iter().position(|n| n == "bootstrap").unwrap();
        let collections_pos = order.iter().position(|n| n == "collections").unwrap();
        let classes_pos = order.iter().position(|n| n == "classes").unwrap();
        assert!(bootstrap_pos < collections_pos);
        assert!(collections_pos < classes_pos);
    }

    #[test]
    fn test_cycle_detection() {
        let result = ModuleGraph::build(vec![
            desc("a", &["b"], &[]),
            desc("b", &["a"], &[]),
        ]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cycle"));
    }

    #[test]
    fn test_missing_dependency() {
        let result = ModuleGraph::build(vec![
            desc("a", &["nonexistent"], &[]),
        ]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown module"));
    }

    #[test]
    fn test_can_remove() {
        let graph = ModuleGraph::build(vec![
            desc("bootstrap", &[], &["Object"]),
            desc("collections", &["bootstrap"], &["Assoc"]),
        ]).unwrap();

        // Can remove leaf
        assert!(graph.can_remove("collections").is_ok());
        // Cannot remove bootstrap (collections depends on it)
        let err = graph.can_remove("bootstrap").unwrap_err();
        assert!(err.contains(&"collections".to_string()));
    }

    #[test]
    fn test_transitive_dependents() {
        let graph = ModuleGraph::build(vec![
            desc("bootstrap", &[], &[]),
            desc("collections", &["bootstrap"], &[]),
            desc("classes", &["collections"], &[]),
            desc("membrane", &["bootstrap"], &[]),
        ]).unwrap();

        let deps = graph.transitive_dependents("bootstrap");
        assert!(deps.contains(&"collections".to_string()));
        assert!(deps.contains(&"classes".to_string()));
        assert!(deps.contains(&"membrane".to_string()));

        let deps = graph.transitive_dependents("collections");
        assert!(deps.contains(&"classes".to_string()));
        assert!(!deps.contains(&"membrane".to_string()));
    }
}
