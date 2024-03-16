# 🛖yurt

[![tests](https://github.com/jcthomassie/yurt/actions/workflows/tests.yaml/badge.svg)](https://github.com/jcthomassie/yurt/actions/workflows/tests.yaml)
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
- `!case` list of conditional steps; breaks after first match
  - `condition` criteria for the case to be accepted
  - `include` build steps to include if the condition is met
- `!link` symlink to be applied
  - `source` link origin path
  - `target` link destination path (real file)
- `!hook` shell command to run on specified actions
  - `on` actions to run the hook on
  - `exec`
    - `shell` shell to run the command with
    - `command` command to run
- `!package` package to install
  - `name` package name
  - `managers` (optional) list of package manager names that provide the package
  - `aliases` (optional) package aliases for specific package managers
- `!package_manager` package manager to bootstrap

Some build steps (such as `require` and `vars`) modify the resolver state.
The order of build steps may change the resolved values.

### Example

```yaml
---
version: "~0.8.0-dev"
build:
  # Require dotfile repo
  - !repo
      path: ~/dotfiles
      url: https://github.com/jcthomassie/dotfiles.git

  # Specify package managers
  - !case
    - condition: !locale { platform: windows }
      include:
        - !package_manager
            name: choco
            shell_install: choco install -y ${{ package.alias }}
            shell_uninstall: choco uninstall -y ${{ package.alias }}
    - condition: !default
      include:
        - !package_manager
            name: brew
            shell_bootstrap: curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh
            shell_has: brew list ${{ package.alias }}
            shell_install: brew install -y ${{ package.alias }}
            shell_uninstall: brew uninstall -y ${{ package.alias }}
        # Run a command with a specific shell
        - !hook
            on: [ install ]
            exec:
              shell: /usr/bin/bash
              command: brew bundle --file ${{ dotfiles.path }}/.brewfile

  # Run a command with the system default shell
  - !hook
      on: [ install, uninstall ]
      exec: |
        echo "doing something"
        echo "doing another thing"

  # Install packages
  - !package { name: bat }
  - !package
      name: git
      managers:
      - apt
      - choco
  - !package
      name: delta
      aliases:
        brew: git-delta
        cargo: git-delta

  # Apply symlinks
  - !link
      target: ${{ repo#dotfiles.path }}/.zsh/.zshrc
      source: ~/.zshrc
  - !link
      target: ${{ repo#dotfiles.path }}/.gitconfig
      source: ~/.gitconfig
```
