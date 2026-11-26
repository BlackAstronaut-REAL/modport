use mojmodelbridge::legacy_api_dictionary::legacy_api_dictionary_for_version;
use serde_json::{Map, Value};

fn main() {
    let mut dictionaries = Map::new();
    for version in ["1.7.10", "1.8.9", "1.12.2"] {
        let entries = legacy_api_dictionary_for_version(version);
        dictionaries.insert(
            version.to_owned(),
            serde_json::to_value(entries).expect("serialize legacy API dictionary"),
        );
    }
    println!(
        "{}",
        serde_json::to_string(&Value::Object(dictionaries))
            .expect("serialize dictionary JSON")
    );
}
