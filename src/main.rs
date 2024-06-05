#[allow(unused_imports)]
use std::io::{self, Write};

fn main() {
    repl();
}

fn repl() {
    let stdin = io::stdin();
    let mut input = String::new();
    loop {
        input.clear();

        // add promt
        print!("$ ");
        io::stdout().flush().unwrap();
        let size = stdin.read_line(&mut input).unwrap();
        if size == 0 {
            break;
        }
        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        println!("{}: command not found", input.trim());
        io::stdout().flush().unwrap();

        // read input
        // process
        // output processed
    }
}
