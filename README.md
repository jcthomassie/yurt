# yurt

[![tests](https://github.com/jcthomassie/yurt/workflows/tests/badge.svg)](https://github.com/jcthomassie/yurt/actions)
[![release](https://github.com/jcthomassie/yurt/workflows/release/badge.svg)](https://github.com/jcthomassie/yurt/releases)

Experimental cross-platform dotfile and package manager.

Build instructions are specified via YAML. Features include symlink application, installation of packages (and package managers), execution of remote shell scripts via curl, and system specific build steps.

## Usage

Install from local build file.

```shell
yurt --yaml=~/build.yaml install
```

Print resolved build steps and exit.

```shell
yurt show
```

## YAML Format

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