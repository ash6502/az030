//! Development aid: assemble every line of a file on its own (at address 0) and print
//! `line-number hex-bytes` or `line-number ERROR message`. Used to compare azas with a
//! reference assembler instruction by instruction.

fn main() {
    let path = std::env::args().nth(1).expect("usage: azas-lines FILE");
    let src = std::fs::read_to_string(&path).expect("read");
    for (i, line) in src.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let prog = format!("\torg 0\n{line}\n");
        let opts = azas::Options { format: azas::Format::Binary, defines: Vec::new() };
        match azas::assemble("line", &prog, opts, &mut azas::NoFiles) {
            Ok(b) => {
                let hex: Vec<String> = b.chunks(2).map(|c| c.iter().map(|x| format!("{x:02X}")).collect()).collect();
                println!("{} {}", i + 1, hex.join(" "));
            }
            Err(e) => println!("{} ERROR {}", i + 1, e.join("; ")),
        }
    }
}
