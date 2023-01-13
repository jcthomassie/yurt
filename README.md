# 🛖yurt

[![tests](https://github.com/jcthomassie/yurt/actions/workflows/tests.yaml/badge.svg)](https://github.com/jcthomassie/yurt/actions/workflows/tests.yaml)
[![build](https://github.com/jcthomassie/yurt/actions/workflows/build.yaml/badge.svg)](https://github.com/jcthomassie/yurt/actions/workflows/build.yaml)
[![release](https://img.shields.io/github/v/release/jcthomassie/yurt?include_prereleases&label=release)](https://github.com/jcthomassie/yurt/releases/latest)

Experimental cross-platform dotfile and package manager.

Build instructions are specified via YAML. Features include symlink application, installation of packages (and package managers), execution of shell commands, and system specific build steps.

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

`version` yurt version for compatibility check (uses [semver](https://docs.rs/semver/latest/semver/index.html))

`build`

- `!repo`
  - `path` local repo directory path
  - `url` remote url for cloning the repo
- `!vars` map of substitution values
- `!matrix` repeat include block for each value
  - `values` values to substitute in the include block
  - `include` build steps to be repeated
- `!case` list of conditional build steps
  - `positive` if the spec matches the local spec
  - `negative` if the spec does not match the local spec
  - `default` if none of the preceeding conditions are met
- `!link` list of symlinks to be applied
- `!run` shell command to run on install
  - `shell` shell to run the command with
  - `command` command to run
- `!install` list of packages to install
  - `name` package name
  - `managers` (optional) list of package managers that provide the package
  - `aliases` (optional) package aliases for specific package managers
- `!require` list of package managers to bootstrap

Some build steps (such as `require` and `vars`) modify the resolver state.
The order of build steps may change the resolved values.

### Example

```yaml
---
version: "~0.5.0"
build:
  # Require dotfile repo
  - !repo
      path: ~/dotfiles
      url: https://github.com/jcthomassie/dotfiles.git

  # Specify package managers
  - !case
    - !positive
        condition: { distro: ubuntu }
        include:
          - !require
            - apt
            - apt-get
    - !positive
        condition: { platform: windows }
        include:
          - !require
            - choco
    - !negative
        condition: { platform: windows }
        include:
          - !require
            - brew
          # Run a command with a specific shell
          - !run
              shell: /usr/bin/bash
              command: brew bundle --file ${{ dotfiles.path }}/.brewfile

  # Run a command with the system default shell
  - !run |
      echo "hello world"

  # Install packages
  - !install
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
  - !link
    - tail: ${{ dotfiles.path }}/.zsh/.zshrc
      head: ~/.zshrc
    - tail: ${{ dotfiles.path }}/.gitconfig
      head: ~/.gitconfig
```
