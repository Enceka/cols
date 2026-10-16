use super::killstreaks::RadarResources;

#[test]
#[ignore = "requires target-build authored aircraft nodes and initialized resources"]
fn captured_afghan_graph_retains_three_ingresses_and_the_native_loop_entry_prefix() {
    let r = RadarResources::load(
        std::path::Path::new(&std::env::var("CODOL_MARKER_RESOURCES").unwrap()),
        "mp_afghan",
        523897474,
    )
    .unwrap();
    assert!(super::mechanisms::Mechanism::radio_available(4, &r));
    assert!(super::mechanisms::Mechanism::radio_available(22, &r));
    assert!(!super::mechanisms::Mechanism::radio_available(2, &r));
    assert!(!super::mechanisms::Mechanism::radio_available(9, &r));
    let paths = r.air_paths.unwrap();
    assert_eq!(paths.nodes.len(), 88);
    for (name, count) in [
        ("heli_start", 3),
        ("heli_loop_start", 1),
        ("heli_leave", 4),
        ("heli_crash_start", 3),
        ("heli_attack_area", 17),
        ("heli_dest", 1),
    ] {
        assert_eq!(paths.entries[name].len(), count);
    }
    for &start in &paths.entries["heli_start"] {
        let route = paths.route(start).unwrap();
        assert!(route.repeat_from.is_none());
        assert_eq!(paths.nodes[*route.nodes.last().unwrap()].name, "auto494");
    }
    let route = paths.route(paths.entries["heli_loop_start"][0]).unwrap();
    assert_eq!(route.repeat_from, Some(1));
    let node = &paths.nodes[route.nodes[1]];
    assert_eq!(node.name, "auto476");
    assert_eq!(node.script_airspeed, Some(40.0));
    assert_eq!(node.script_accel, Some(15.0));
}
