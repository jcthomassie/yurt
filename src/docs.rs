/// Import an example YAML file into doc comment
#[macro_export]
macro_rules! yaml_example {
    ($path:literal) => {
        concat!("\n#Example\n", "```yaml\n", include_str!($path), "```\n")
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
                serde_yaml::from_str::<$yaml_type>(include_str!(concat!(
                    "../examples/",
                    stringify!($name),
                    ".yaml"
                )))
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
