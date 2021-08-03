# yurt

[![tests](https://github.com/jcthomassie/yurt/actions/workflows/tests.yaml/badge.svg)](https://github.com/jcthomassie/yurt/actions/workflows/tests.yaml)
[![publish](https://github.com/jcthomassie/yurt/actions/workflows/publish.yaml/badge.svg?event=release)](https://github.com/jcthomassie/yurt/actions/workflows/publish.yaml)

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
`repo`
- `local` path to local dotfile repository
- `remote` dotfile remote url for cloning

`build`
- `case` list of conditional build steps
- `install` list of packages to install
- `bootstrap` list of package managers to bootstrap
- `link` list of symlinks to be applied

### Example
```yaml
---
repo:
  local: $HOME/dotfiles
  remote: https://github.com/jcthomassie/dotfiles.git

build:
  # Bootstrap package managers
  - case:
    - local:
        spec:
          platform: windows
        build:
          - bootstrap:
            - choco
    - default:
        build:
          - bootstrap:
            - brew

  - bootstrap:
    - cargo

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
    - tail: $YURT_REPO_LOCAL/.zsh/.zshrc
      head: $HOME/.zshrc
    - tail: $YURT_REPO_LOCAL/.gitconfig
      head: $HOME/.gitconfig
```
