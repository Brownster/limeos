use super::*;

fn decode(text: &[u8], list: bool) -> Result<Json> {
    let admission = Admission::new(EngineReadProfile::CombinedV1);
    Json::decode(text, admission.response_scope(RESPONSE)?, list)
}

#[test]
fn bounded_value_matches_standard_semantics() {
    let bytes =
        br#"{"unknown":{"a":[true,false,null,-1,2,1.25,"\u20ac"]},"duplicate":1,"duplicate":2}"#;
    assert_eq!(
        *decode(bytes, false).unwrap(),
        serde_json::from_slice::<Value>(bytes).unwrap()
    );
}

#[test]
fn complete_list_boundary_is_checked_before_next_value_storage() {
    let admission = Admission::new(EngineReadProfile::CombinedV1);
    let sixteen = serde_json::to_vec(&vec![0; CONSUMERS]).unwrap();
    assert!(Json::decode(&sixteen, admission.response_scope(RESPONSE).unwrap(), true).is_ok());
    let seventeen = serde_json::to_vec(&vec![0; CONSUMERS + 1]).unwrap();
    assert!(
        Json::decode(
            &seventeen,
            admission.response_scope(RESPONSE).unwrap(),
            true
        )
        .is_err()
    );
    assert_eq!(admission.0.live.load(Ordering::Relaxed), LIVE_BYTES);
}

#[test]
fn unknown_nested_and_escaped_payloads_are_not_free() {
    let string = format!(r#"{{"unknown":"{}"}}"#, "\\n".repeat(STRING + 1));
    assert!(decode(string.as_bytes(), false).is_err());
    let nested = format!("{}0{}", "[".repeat(DEPTH), "]".repeat(DEPTH));
    assert!(decode(nested.as_bytes(), false).is_ok());
    let deeper = format!("[{nested}]");
    assert!(decode(deeper.as_bytes(), false).is_err());
    let map = serde_json::to_vec(
        &(0..ITEMS + 1)
            .map(|i| (i.to_string(), i))
            .collect::<std::collections::BTreeMap<_, _>>(),
    )
    .unwrap();
    assert!(decode(&map, false).is_err());
}

#[test]
fn live_scratch_and_retained_reservations_overlap_and_release() {
    let admission = Admission::new(EngineReadProfile::CombinedV1);
    let mut retained = admission.scope();
    retained.charge(LIVE_BYTES - RESPONSE).unwrap();
    assert!(admission.response_scope(RESPONSE).is_err());
    assert_eq!(admission.0.live.load(Ordering::Relaxed), RESPONSE);
    drop(retained);
    let json = Json::decode(
        br#"{"escaped":"\u20ac"}"#,
        admission.response_scope(RESPONSE).unwrap(),
        false,
    )
    .unwrap();
    assert!(admission.0.live.load(Ordering::Relaxed) < LIVE_BYTES);
    drop(json);
    assert_eq!(admission.0.live.load(Ordering::Relaxed), LIVE_BYTES);
}

#[test]
fn byte_budget_never_renews_and_parallel_collection_is_refused() {
    let admission = Admission::new(EngineReadProfile::CombinedV1);
    admission.consume(BODY_BYTES).unwrap();
    assert!(admission.consume(1).is_err());
    let first = admission.collect().unwrap();
    assert!(admission.collect().is_err());
    drop(first);
    assert!(admission.collect().is_ok());
    assert!(admission.consume(1).is_err());
}

#[test]
fn overspend_exhausts_residual_allowance_from_failed_attempt() {
    let admission = Admission::new(EngineReadProfile::CombinedV1);
    admission.consume(BODY_BYTES - 10).unwrap();
    assert!(admission.consume(11).is_err());
    assert!(admission.consume(1).is_err());
    assert!(admission.ensure_body().is_err());
    assert_eq!(admission.0.body.load(Ordering::Relaxed), 0);
}

#[test]
fn exact_string_map_sequence_limits_and_unknown_node_amplification() {
    let text = format!("\"{}\"", "\\n".repeat(STRING));
    assert!(decode(text.as_bytes(), false).is_ok());
    let map = serde_json::to_vec(
        &(0..ITEMS)
            .map(|i| (i.to_string(), i))
            .collect::<std::collections::BTreeMap<_, _>>(),
    )
    .unwrap();
    assert!(decode(&map, false).is_ok());
    let sequence = serde_json::to_vec(&vec![0; ITEMS]).unwrap();
    assert!(decode(&sequence, false).is_ok());
    assert!(decode(&serde_json::to_vec(&vec![0; ITEMS + 1]).unwrap(), false).is_err());
    let nodes = serde_json::to_vec(&vec![vec![0; ITEMS]; 8]).unwrap();
    let admission = Admission::new(EngineReadProfile::CombinedV1);
    let mut scope = admission.response_scope(RESPONSE).unwrap();
    let mut state = Decode {
        scope: &mut scope,
        nodes: 0,
        strings: 0,
    };
    let mut decoder = serde_json::Deserializer::from_slice(&nodes);
    assert!(
        Seed {
            state: &mut state,
            depth: 0,
            array_limit: ITEMS
        }
        .deserialize(&mut decoder)
        .is_err()
    );
    assert_eq!(state.nodes, NODES);
}

#[test]
fn unicode_scratch_reuse_long_numbers_and_late_escape_release_reservations() {
    let admission = Admission::new(EngineReadProfile::CombinedV1);
    let unicode = format!(
        "{{\"one\":\"{}\",\"two\":\"{}\"}}",
        "\\u20ac".repeat(4096),
        "\\u20ac".repeat(4096)
    );
    assert!(unicode.len() <= RESPONSE);
    let value = Json::decode(
        unicode.as_bytes(),
        admission.response_scope(RESPONSE).unwrap(),
        false,
    )
    .unwrap();
    assert_eq!(value["one"].as_str().unwrap().len(), 3 * 4096);
    assert_eq!(
        *value,
        serde_json::from_slice::<Value>(unicode.as_bytes()).unwrap()
    );
    drop(value);
    assert_eq!(admission.remaining_live(), LIVE_BYTES);
    let number = format!("1.{}e-1", "1".repeat(16 * 1024));
    let value = Json::decode(
        number.as_bytes(),
        admission.response_scope(RESPONSE).unwrap(),
        false,
    )
    .unwrap();
    assert_eq!(
        *value,
        serde_json::from_slice::<Value>(number.as_bytes()).unwrap()
    );
    drop(value);
    let late_escape = format!("\"{}\\n\"", "x".repeat(RESPONSE - 16));
    assert!(late_escape.len() <= RESPONSE);
    assert!(
        Json::decode(
            late_escape.as_bytes(),
            admission.response_scope(RESPONSE).unwrap(),
            false
        )
        .is_err()
    );
    assert_eq!(admission.remaining_live(), LIVE_BYTES);
}

#[test]
fn excess_nested_item_or_escaped_key_is_not_read_by_decoder() {
    use std::{
        cell::Cell,
        io::{self, Read},
    };
    struct Counted<'a> {
        bytes: &'a [u8],
        count: &'a Cell<usize>,
    }
    impl Read for Counted<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let n = output.len().min(self.bytes.len());
            output[..n].copy_from_slice(&self.bytes[..n]);
            self.bytes = &self.bytes[n..];
            self.count.set(self.count.get() + n);
            Ok(n)
        }
    }
    let array_prefix = format!("[{},", vec!["0"; CONSUMERS].join(","));
    let array = format!(
        "{}{{\"nested\":[\"{}\"]}}]",
        array_prefix,
        "\\u0061".repeat(1000)
    );
    let map_prefix = format!(
        "{{{},",
        (0..ITEMS)
            .map(|i| format!("\"{i}\":0"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let map = format!("{}\"{}\":null}}", map_prefix, "\\u0061".repeat(1000));
    for (bytes, prefix, limit) in [
        (array.as_bytes(), array_prefix.len(), CONSUMERS),
        (map.as_bytes(), map_prefix.len(), ITEMS),
    ] {
        let admission = Admission::new(EngineReadProfile::CombinedV1);
        let mut scope = admission.response_scope(RESPONSE).unwrap();
        let count = Cell::new(0);
        let mut decoder = serde_json::Deserializer::from_reader(Counted {
            bytes,
            count: &count,
        });
        let mut state = Decode {
            scope: &mut scope,
            nodes: 0,
            strings: 0,
        };
        assert!(
            Seed {
                state: &mut state,
                depth: 0,
                array_limit: limit
            }
            .deserialize(&mut decoder)
            .is_err()
        );
        assert!(
            count.get() <= prefix + 1,
            "excess content was decoded: {} bytes after prefix {}",
            count.get(),
            prefix
        );
    }
}
