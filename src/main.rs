mod layout;
mod linear;
mod model;
mod ops;
mod parser;
mod ui;

use std::env;
use std::fs;
use std::path::Path;

const DEFAULT_MAP_FILE: &str = "mind.hmm";

fn main() {
    let args: Vec<String> = env::args().collect();

    let (mm, filename) = if args.len() > 1 {
        let path = &args[1];
        match fs::read_to_string(path) {
            Ok(content) => {
                let mm = parser::parse(&content);
                (mm, Some(path.clone()))
            }
            Err(e) => {
                eprintln!("Error reading {}: {}", path, e);
                std::process::exit(1);
            }
        }
    } else {
        let path = DEFAULT_MAP_FILE.to_string();
        if Path::new(&path).exists() {
            match fs::read_to_string(&path) {
                Ok(content) => {
                    let mm = parser::parse(&content);
                    (mm, Some(path))
                }
                Err(e) => {
                    eprintln!("Error reading {}: {}", path, e);
                    std::process::exit(1);
                }
            }
        } else {
            (model::MindMap::new("root"), Some(path))
        }
    };

    let app = ui::App::new(mm, filename);

    if let Err(e) = ui::run(app) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
