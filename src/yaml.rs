use serde::Deserialize;
use serde_yaml::Value;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

pub fn parse(path: PathBuf) -> std::io::Result<()> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    for document in serde_yaml::Deserializer::from_reader(reader) {
        let value = Value::deserialize(document);
        println!("{:?}", value);
    }
    Ok(())
}
