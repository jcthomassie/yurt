# 🛖yurt

[![tests](https://github.com/jcthomassie/yurt/actions/workflows/tests.yaml/badge.svg)](https://github.com/jcthomassie/yurt/actions/workflows/tests.yaml)
[![build](https://github.com/jcthomassie/yurt/actions/workflows/build.yaml/badge.svg?event=release)](https://github.com/jcthomassie/yurt/actions/workflows/build.yaml)
[![release](https://img.shields.io/github/v/release/jcthomassie/yurt?include_prereleases&label=release)](https://github.com/jcthomassie/yurt/releases/latest)

Experimental cross-platform dotfile and package manager.

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
  - `local` path to local dotfile repository
  - `remote` dotfile remote url for cloning
- `case` list of conditional build steps
  - `positive` if the spec matches the local spec
  - `negative` if the spec does not match the local spec
  - `default` if none of the preceeding conditions are met
- `link` list of symlinks to be applied
- `install` list of packages to install
  - `name` package name
  - `alias` package alias for package managers
  - `managers` list of package managers that provide the package
- `require` list of package managers to bootstrap
- `bundle`
  - `manager` single package manager
  - `packages` list of package names to install

### Example

```yaml
---
version: 0.1.0

build:
  # Require dotfile repo
  - repo:
      local: ~/dotfiles
      remote: https://github.com/jcthomassie/dotfiles.git
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
    - name: zsh
      managers:
      - brew
      - apt-get
    - name: git
      managers:
      - brew
      - apt
      - choco

  # Apply symlinks
  - link:
    - tail: ${{ dotfiles.local }}/.zsh/.zshrc
      head: ~/.zshrc
    - tail: ${{ dotfiles.local }}/.gitconfig
      head: ~/.gitconfig
```
