//! Actual record-set frontier, preserving the root members without rereading it.
use crate::closure_node::Digest;
use serde_json::Value;

pub trait Source: crate::closure::SourceModel<Reference = Value> {
    type Error;

    fn node(
        &mut self,
        reference: &Value,
        task: Option<&str>,
        attempt: Option<&str>,
    ) -> Result<(Vec<Value>, Vec<Value>), Self::Error>;
}

pub enum Error<R, D> {
    Read(R),
    Digest(D),
}

#[allow(unused_variables, clippy::len_zero)]
pub fn validate<S: Source, D: Digest>(
    source: &mut S,
    digester: &mut D,
    reference: &Value,
    task: Option<&str>,
    attempt: Option<&str>,
) -> Result<Vec<Value>, Error<S::Error, D::Error>> {
    let root_key = match digester.digest(reference) {
        Ok(key) => key,
        Err(e) => return Err(Error::Digest(e)),
    };

    let mut frontier = crate::traversal::start();
    crate::traversal::schedule(&mut frontier, root_key, (reference.clone(), true));
    let mut root_records = Vec::new();

    loop {
        let Some((key, (value, is_root))) = crate::traversal::next(&mut frontier) else {
            return Ok(root_records);
        };

        let input = if is_root { reference } else { &value };
        let (records, mut dependencies) = match source.node(
            input,
            if is_root { task } else { None },
            if is_root { attempt } else { None },
        ) {
            Ok(node) => node,
            Err(e) => return Err(Error::Read(e)),
        };
        if is_root {
            root_records = records;
        }

        while dependencies.len() > 0 {
            let dependency = dependencies.pop().unwrap();
            let child = match digester.digest(&dependency) {
                Ok(key) => key,
                Err(e) => return Err(Error::Digest(e)),
            };

            crate::traversal::schedule(&mut frontier, child, (dependency, false));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    struct Names;
    impl Digest for Names {
        type Error = &'static str;
        fn digest(&mut self, value: &Value) -> Result<String, Self::Error> {
            value.as_str().map(str::to_owned).ok_or("bad reference")
        }
    }
    struct Nodes {
        nodes: HashMap<String, (Vec<Value>, Vec<Value>)>,
        visits: Vec<(String, Option<String>, Option<String>)>,
    }
    impl crate::closure::SourceModel for Nodes {
        type Reference = Value;
    }

    impl Source for Nodes {
        type Error = &'static str;
        fn node(
            &mut self,
            reference: &Value,
            task: Option<&str>,
            attempt: Option<&str>,
        ) -> Result<(Vec<Value>, Vec<Value>), Self::Error> {
            let key = reference.as_str().unwrap().to_owned();
            self.visits.push((
                key.clone(),
                task.map(str::to_owned),
                attempt.map(str::to_owned),
            ));
            self.nodes.get(&key).cloned().ok_or("unreadable node")
        }
    }
    #[test]
    fn cyclic_shared_dependencies_preserve_root_members_and_single_visits() {
        let mut source = Nodes {
            nodes: HashMap::from([
                (
                    "root".into(),
                    (
                        vec![json!("root-record")],
                        vec![json!("left"), json!("right")],
                    ),
                ),
                (
                    "left".into(),
                    (
                        vec![json!("left-record")],
                        vec![json!("root"), json!("shared")],
                    ),
                ),
                (
                    "right".into(),
                    (vec![json!("right-record")], vec![json!("shared")]),
                ),
                ("shared".into(), (vec![], vec![])),
            ]),
            visits: vec![],
        };
        let result = validate(
            &mut source,
            &mut Names,
            &json!("root"),
            Some("task"),
            Some("attempt"),
        )
        .ok()
        .unwrap();
        assert_eq!(result, vec![json!("root-record")]);
        assert_eq!(
            source.visits,
            vec![
                ("root".into(), Some("task".into()), Some("attempt".into())),
                ("left".into(), None, None),
                ("shared".into(), None, None),
                ("right".into(), None, None),
            ]
        );
    }
    #[test]
    fn failed_dependency_never_returns_partial_root_members() {
        let mut source = Nodes {
            nodes: HashMap::from([
                (
                    "root".into(),
                    (
                        vec![json!("root-record")],
                        vec![json!("missing"), json!("later")],
                    ),
                ),
                ("later".into(), (vec![], vec![])),
            ]),
            visits: vec![],
        };
        assert!(matches!(
            validate(&mut source, &mut Names, &json!("root"), None, None),
            Err(Error::Read("unreadable node"))
        ));
        assert_eq!(
            source.visits,
            vec![("root".into(), None, None), ("missing".into(), None, None)]
        );
    }
    #[test]
    fn leaf_root_is_read_once_and_returns_its_original_members() {
        let mut source = Nodes {
            nodes: HashMap::from([("root".into(), (vec![json!(1), json!(2)], vec![]))]),
            visits: vec![],
        };
        assert_eq!(
            validate(&mut source, &mut Names, &json!("root"), None, None).ok(),
            Some(vec![json!(1), json!(2)])
        );
        assert_eq!(source.visits.len(), 1);
    }
}
