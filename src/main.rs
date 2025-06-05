mod args;
mod repl;
mod builtin;
mod redirect;
mod completion;
mod terminal;

use repl::repl;

fn term() -> anyhow::Result<i32> {
    let mut exit_code = 0;
    match repl() {
        Ok(Some(e)) => {
            exit_code = e;
        }
        Ok(None) => {}
        Err(err) => {
            println!("Error: {:?}", err);
        }
    }

    Ok(exit_code)
}

fn main() -> anyhow::Result<()> {
    match term() {
        Ok(v) => {
            std::process::exit(v);
        }
        Err(e) => Err(e)?,
    }
}
