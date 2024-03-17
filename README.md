# 🛖yurt

[![tests](https://github.com/jcthomassie/yurt/actions/workflows/tests.yaml/badge.svg)](https://github.com/jcthomassie/yurt/actions/workflows/tests.yaml)
[![docs](https://github.com/jcthomassie/yurt/actions/workflows/docs.yaml/badge.svg)](https://github.com/jcthomassie/yurt/actions/workflows/docs.yaml)
[![release](https://img.shields.io/github/v/release/jcthomassie/yurt?include_prereleases&label=release)](https://github.com/jcthomassie/yurt/releases/latest)

Experimental cross-platform dotfile and package manager.

Build instructions are specified via YAML. Features include symlink application, installation of packages (and package managers), execution of shell commands, and system specific build steps.

## Usage

Install from local build file:

```shell
yurt --file "~/build.yaml" install
```

Install from remote build file:

```shell
yurt --file-url "https://raw.githubusercontent.com/jcthomassie/dotfiles/HEAD/build.yaml" install
```

Print resolved build steps and exit:

```shell
yurt show
```

**Note:** Default build path is specified via the `YURT_BUILD_FILE` environment variable.
