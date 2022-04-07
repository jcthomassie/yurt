# 🛖yurt

[![tests](https://github.com/jcthomassie/yurt/actions/workflows/tests.yaml/badge.svg)](https://github.com/jcthomassie/yurt/actions/workflows/tests.yaml)
[![build](https://github.com/jcthomassie/yurt/actions/workflows/build.yaml/badge.svg?event=release)](https://github.com/jcthomassie/yurt/actions/workflows/build.yaml)
[![release](https://img.shields.io/github/v/release/jcthomassie/yurt?include_prereleases&label=release)](https://github.com/jcthomassie/yurt/releases/latest)

Experimental cross-platform dotfile and package manager wrapper.

Build instructions are specified via YAML. Features include symlink application, installation of packages (and package managers), execution of remote shell scripts via curl, and system specific build steps.

## Usage

Install from local build file:

```shell
yurt --yaml "~/build.yaml" install
```

Install from remote build file:

```shell
yurt --yaml-url "https://raw.githubusercontent.com/jcthomassie/dotfiles/HEAD/build.yaml" install
```

Print resolved build steps and exit:

```shell
yurt show
```

**Note:** Default build path is specified via the `YURT_BUILD_FILE` environment variable.

## Build Format

Build parameters are specified via a YAML file. Cases can be arbitrarily nested. Order of build steps is preserved after resolution.

### Fields

`version` yurt version for compatibility check

`shell` set the shell for POSIX systems

`build`

- `repo`
  - `path` local repo directory path
  - `url` remote url for cloning the repo
- `namespace` map of substitution values
  - `name` name of the namespace
  - `values` variable definitions
- `matrix` repeat include block for each value
  - `values` values to substitute in the include block
  - `include` build steps to be repeated
- `case` list of conditional build steps
  - `positive` if the spec matches the local spec
  - `negative` if the spec does not match the local spec
  - `default` if none of the preceeding conditions are met
- `link` list of symlinks to be applied
- `install` list of packages to install
  - `name` package name
  - `managers` (optional) list of package managers that provide the package
  - `aliases` (optional) package aliases for specific package managers
- `require` list of package managers to bootstrap

Some build steps (such as `require` and `namespace`) modify the resolver state.
The order of build steps may change the resolved values.

### Example

```yaml
---
version: 0.2.1
shell: zsh
build:
  # Require dotfile repo
  - repo:
      path: ~/dotfiles
      url: https://github.com/jcthomassie/dotfiles.git

  # Specify package managers
  - case:
    - positive:
        spec: { distro: ubuntu }
        include:
          - require:
            - apt
            - apt-get
    - positive:
        spec: { platform: windows }
        include:
          - require:
            - choco
    - negative:
        spec: { platform: windows }
        include:
          - require:
            - brew

  # Install packages
  - install:
    - name: bat
    - name: git
      managers:
      - apt
      - choco
    - name: delta
      aliases:
        brew: git-delta
        cargo: git-delta

  # Apply symlinks
  - link:
    - tail: ${{ dotfiles.path }}/.zsh/.zshrc
      head: ~/.zshrc
    - tail: ${{ dotfiles.path }}/.gitconfig
      head: ~/.gitconfig
```
