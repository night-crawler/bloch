use std::borrow::Cow;
#[cfg(not(feature = "serde1"))]
use std::collections::HashSet;
#[cfg(feature = "serde1")]
use std::{
    collections::{HashMap, hash_map::Entry},
    ops::Range,
    sync::OnceLock,
};

use crate::{Error, mark::Mark, value::Value};

#[derive(Debug)]
pub struct Json<'a> {
    pub(crate) paths: Box<[Cow<'a, str>]>,
    // Columns follow path order so a path index also selects its values.
    pub(crate) columns: Box<[Mark<'a>]>,
    num_typed_paths: usize,
    num_rows: usize,
    #[cfg(feature = "serde1")]
    nodes: Box<[JsonPathNode]>,
}

#[cfg(feature = "serde1")]
#[derive(Debug)]
struct JsonPathNode {
    // The root has no path segment; every inserted child has a span.
    key: Option<KeySpan>,
    decoded_key: Option<OnceLock<String>>,
    leaf: Option<usize>,
    first_child: Option<usize>,
    next_sibling: Option<usize>,
}

#[cfg(feature = "serde1")]
impl JsonPathNode {
    fn key<'a>(&'a self, paths: &'a [Cow<'_, str>], decode: bool) -> &'a str {
        let key = self
            .key
            .as_ref()
            .map_or("", |span| &paths[span.path][span.bytes.clone()]);
        if decode && let Some(decoded) = &self.decoded_key {
            decoded.get_or_init(|| key.replace("%2E", ".")).as_str()
        } else {
            key
        }
    }
}

// Escaped paths may own their text. Store segment coordinates rather than references
// into that text so the tree stays movable without copying each segment.
#[cfg(feature = "serde1")]
#[derive(Debug)]
struct KeySpan {
    path: usize,
    bytes: Range<usize>,
}

impl<'a> Json<'a> {
    pub(crate) fn new(
        paths: Vec<Cow<'a, str>>,
        columns: Vec<Mark<'a>>,
        num_typed_paths: usize,
        rows: usize,
    ) -> crate::Result<Self> {
        if paths.len() != columns.len() {
            return Err(Error::CorruptedData(format!(
                "JSON has {} paths but {} columns",
                paths.len(),
                columns.len()
            )));
        }

        #[cfg(not(feature = "serde1"))]
        {
            let mut unique_paths = HashSet::with_capacity(paths.len());
            for path in &paths {
                if !unique_paths.insert(path.as_ref()) {
                    return Err(Error::CorruptedData(format!(
                        "duplicate JSON path {path:?}"
                    )));
                }
            }
        }

        #[cfg(feature = "serde1")]
        let nodes = {
            let mut tree = PathTree::new();
            for (path_index, path) in paths.iter().enumerate() {
                tree.insert_path(path_index, path)?;
            }
            tree.nodes.into_boxed_slice()
        };

        Ok(Self {
            paths: paths.into_boxed_slice(),
            columns: columns.into_boxed_slice(),
            num_typed_paths,
            num_rows: rows,
            #[cfg(feature = "serde1")]
            nodes,
        })
    }

    #[cfg(feature = "serde1")]
    pub(crate) const fn root(&self) -> usize {
        PathTree::ROOT
    }

    #[cfg(feature = "serde1")]
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    #[cfg(feature = "serde1")]
    pub(crate) fn node_key(&'a self, node: usize, decode: bool) -> &'a str {
        self.nodes[node].key(&self.paths, decode)
    }

    #[cfg(feature = "serde1")]
    pub(crate) fn node_leaf(&self, node: usize) -> Option<usize> {
        self.nodes[node].leaf
    }

    #[cfg(feature = "serde1")]
    pub(crate) fn first_child(&self, node: usize) -> Option<usize> {
        self.nodes[node].first_child
    }

    #[cfg(feature = "serde1")]
    pub(crate) fn next_sibling(&self, node: usize) -> Option<usize> {
        self.nodes[node].next_sibling
    }

    pub(crate) const fn len(&self) -> usize {
        self.num_rows
    }

    pub(crate) const fn get(&'a self, row: usize) -> Option<Value<'a>> {
        if row < self.num_rows {
            Some(Value::Json {
                mark: self,
                index: row,
            })
        } else {
            None
        }
    }

    pub(crate) fn value(
        &'a self,
        path_index: usize,
        row: usize,
    ) -> crate::Result<Option<Value<'a>>> {
        let Some(column) = self.columns.get(path_index) else {
            return Err(Error::CorruptedData(format!(
                "JSON path index {path_index} has no column"
            )));
        };
        if self.is_absent(path_index, row) {
            return Ok(None);
        }
        column.get(row)
    }

    pub(crate) fn is_absent(&self, path_index: usize, row: usize) -> bool {
        // Only an untyped JSON path uses a Dynamic NULL to represent a missing key.
        path_index >= self.num_typed_paths
            && matches!(self.columns.get(path_index), Some(Mark::Dynamic(dynamic)) if dynamic.is_null(row))
    }
}

#[cfg(feature = "serde1")]
struct PathTree<'a> {
    nodes: Vec<JsonPathNode>,
    // These indexes are needed only while inserting paths; discard them afterward
    // so traversal retains neither hash tables nor borrows into owned path text.
    children: HashMap<(usize, &'a str), usize>,
    last_child: Vec<Option<usize>>,
}

#[cfg(feature = "serde1")]
impl<'a> PathTree<'a> {
    const ROOT: usize = 0;

    fn new() -> Self {
        Self {
            nodes: vec![JsonPathNode {
                key: None,
                decoded_key: None,
                leaf: None,
                first_child: None,
                next_sibling: None,
            }],
            children: HashMap::new(),
            last_child: vec![None],
        }
    }

    fn insert_path(&mut self, path_index: usize, path: &'a str) -> crate::Result<()> {
        let mut parent = Self::ROOT;
        let mut start = 0;
        for key in path.split('.') {
            let end = start + key.len();
            parent = self.child(
                parent,
                KeySpan {
                    path: path_index,
                    bytes: start..end,
                },
                key,
            );
            start = end + '.'.len_utf8();
        }

        if self.nodes[parent].leaf.replace(path_index).is_some() {
            return Err(Error::CorruptedData(format!(
                "duplicate JSON path {path:?}"
            )));
        }
        Ok(())
    }

    // Every new node is assigned `nodes.len()` as its index before any of its own
    // children can be created, so a child's index always exceeds its parent's — callers
    // may process `nodes` in reverse to resolve every child before its parent.
    fn child(&mut self, parent: usize, span: KeySpan, key: &'a str) -> usize {
        match self.children.entry((parent, key)) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let index = self.nodes.len();
                self.nodes.push(JsonPathNode {
                    key: Some(span),
                    decoded_key: key.contains("%2E").then(OnceLock::new),
                    leaf: None,
                    first_child: None,
                    next_sibling: None,
                });
                self.last_child.push(None);

                if let Some(sibling) = self.last_child[parent] {
                    self.nodes[sibling].next_sibling = Some(index);
                } else {
                    self.nodes[parent].first_child = Some(index);
                }
                self.last_child[parent] = Some(index);
                entry.insert(index);
                index
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicate_raw_paths() {
        let result = Json::new(
            vec!["nested.key".into(), "other".into(), "nested.key".into()],
            vec![Mark::Nothing(1), Mark::Nothing(1), Mark::Nothing(1)],
            1,
            1,
        );
        assert!(matches!(result, Err(Error::CorruptedData(_))));
    }

    #[test]
    fn accepts_prefix_paths_in_either_order() {
        for paths in [
            vec!["a".into(), "a.b".into()],
            vec!["a.b".into(), "a".into()],
        ] {
            let json = Json::new(paths, vec![Mark::Nothing(1), Mark::Empty], 2, 1).unwrap();
            assert!(matches!(json.value(0, 0).unwrap(), Some(Value::Empty)));
            assert!(json.value(1, 0).unwrap().is_none());
        }
    }

    #[cfg(feature = "serde1")]
    #[test]
    fn preserves_first_seen_sibling_order() {
        fn children<'a>(json: &'a Json<'a>, parent: usize) -> Vec<(&'a str, Option<usize>)> {
            let mut result = Vec::new();
            let mut child = json.first_child(parent);
            while let Some(node) = child {
                result.push((json.node_key(node, false), json.node_leaf(node)));
                child = json.next_sibling(node);
            }
            result
        }

        let paths = ["z.b", "a.b", "z.a", "z", "a.c", "m", "a.b.d"]
            .into_iter()
            .map(Cow::Borrowed)
            .collect::<Vec<_>>();
        let columns = paths.iter().map(|_| Mark::Nothing(1)).collect();
        let json = Json::new(paths, columns, 7, 1).unwrap();

        assert_eq!(
            children(&json, json.root()),
            vec![("z", Some(3)), ("a", None), ("m", Some(5))]
        );
        let z = json.first_child(json.root()).unwrap();
        assert_eq!(children(&json, z), vec![("b", Some(0)), ("a", Some(2))]);
        let a = json.next_sibling(z).unwrap();
        assert_eq!(children(&json, a), vec![("b", Some(1)), ("c", Some(4))]);
        let b = json.first_child(a).unwrap();
        assert_eq!(children(&json, b), vec![("d", Some(6))]);
    }
}
