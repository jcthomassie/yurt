/// Import an example YAML file
#[macro_export]
macro_rules! yaml_example_str {
    ($name:ident) => {
        yaml_example_str!(concat!(stringify!($name), ".yaml"))
    };

    ($name:expr) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/", $name))
    };
}

/// Import an example YAML file formatted for a doc comment
#[macro_export]
macro_rules! yaml_example_doc {
    ($name:literal) => {
        concat!(
            "\n\n# Example\n\n",
            "```yaml\n",
            $crate::yaml_example_str!($name),
            "\n```\n"
        )
    };
}

#[cfg(test)]
pub mod tests {
    use crate::config::Config;
    use crate::specs::BuildSpec;

    macro_rules! test_case {
        ($name:ident, $yaml_type:ty) => {
            #[test]
            fn $name() {
                serde_yaml::from_str::<$yaml_type>(yaml_example_str!($name))
                    .expect("Failed to parse yaml");
            }
        };
    }

    test_case!(config, Config);
    test_case!(case, BuildSpec);
    test_case!(hook, BuildSpec);
    test_case!(link, BuildSpec);
    test_case!(matrix, BuildSpec);
    test_case!(package_manager, BuildSpec);
    test_case!(package, BuildSpec);
    test_case!(repo, BuildSpec);
    test_case!(vars, BuildSpec);
}
