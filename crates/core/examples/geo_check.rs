//! Compare our EWKB decoding with PostGIS: reads `hex|wkt` lines on stdin.
fn main() {
    let mut bad = 0;
    for line in std::io::stdin().lines().map_while(Result::ok) {
        let Some((hex, wkt)) = line.split_once('|') else { continue };
        let ours = rgrip_core::geo::parse_hex(hex).map(|g| g.geometry.to_wkt());
        let norm = |s: &str| s.replace(", ", ",").replace(" (", "(");
        let ok = ours.as_deref().map(norm) == Some(norm(wkt));
        if !ok {
            bad += 1;
            println!("MISMATCH\n  postgis: {wkt}\n  ours:    {ours:?}");
        }
    }
    println!("{bad} mismatches");
}
