pub const TARGET: &str = "adnl";

pub fn dump(m: &str, d: &[u8]) {
    let mut dump = String::new();
    for i in 0..d.len() {
        dump.push_str(&format!("{:02x}{}", d[i], if (i + 1) % 16 == 0 { '\n' } else { ' ' }));
    }
    debug!(target: TARGET, "{}:\n{}", m, dump);
}
