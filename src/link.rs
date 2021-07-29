use super::error::DotsResult;
use serde::Deserialize;
use shellexpand;
use std::io::{Error, ErrorKind};
use std::path::PathBuf;
use symlink;

#[inline]
pub fn expand_path<S: ?Sized + AsRef<str>>(path: &S) -> DotsResult<PathBuf> {
    _expand_path(path.as_ref())
}

#[inline]
fn _expand_path(path: &str) -> DotsResult<PathBuf> {
    Ok(PathBuf::from(shellexpand::full(path)?.as_ref()))
}

pub enum LinkStatus {
    Exists,
    NotExists,
    Invalid(Error),
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Link {
    // head@ -> tail
    head: PathBuf,
    tail: PathBuf,
}

impl Link {
    fn _new(head: PathBuf, tail: PathBuf) -> Self {
        Self {
            head: head,
            tail: tail,
        }
    }

    pub fn new<P: Into<PathBuf>>(head: P, tail: P) -> Self {
        Self::_new(head.into(), tail.into())
    }

    // Performs shell expansion on input paths
    pub fn expand<S: ?Sized + AsRef<str>>(head: &S, tail: &S) -> DotsResult<Self> {
        Ok(Self::_new(expand_path(head)?, expand_path(tail)?))
    }

    // Get current status of link
    pub fn status(&self) -> LinkStatus {
        if !self.tail.exists() {
            return LinkStatus::Invalid(Error::new(
                ErrorKind::NotFound,
                "link target does not exist",
            ));
        }
        if !self.head.exists() {
            return LinkStatus::NotExists;
        }
        match self.head.read_link() {
            Ok(target) if target == self.tail => LinkStatus::Exists,
            Ok(target) => LinkStatus::Invalid(Error::new(
                ErrorKind::AlreadyExists,
                format!("link source points to wrong target: {:?}", target),
            )),
            Err(e) => LinkStatus::Invalid(e),
        }
    }

    // Try to create link if it does not already exist
    pub fn link(&self) -> std::io::Result<()> {
        match self.status() {
            LinkStatus::Exists => Ok(()),
            LinkStatus::NotExists => {
                println!("Linking {:?}@->{:?}", &self.head, &self.tail);
                symlink::symlink_file(&self.tail, &self.head)
            }
            LinkStatus::Invalid(e) => Err(e),
        }
    }

    // Try to remove link if it exists
    pub fn unlink(&self) -> std::io::Result<()> {
        match self.status() {
            LinkStatus::Exists => {
                println!("Unlinking {:?}@->{:?}", &self.head, &self.tail);
                symlink::remove_symlink_file(&self.head)
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile;

    #[test]
    fn status() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let ln = Link::new(dir.path().join("link.head"), dir.path().join("link.tail"));
        // Neither end exists
        assert!(matches!(ln.status(), LinkStatus::Invalid(_)));
        // Head does not exist
        File::create(&ln.tail).expect("failed to create tempfile");
        assert!(matches!(ln.status(), LinkStatus::NotExists));
        // Head links to tail
        symlink::symlink_file(&ln.tail, &ln.head).expect("failed to create symlink");
        assert!(matches!(ln.status(), LinkStatus::Exists));
        symlink::remove_symlink_file(&ln.head).expect("failed to remove symlink");
        // Head links to wrong file
        let wrong = dir.path().join("wrong.thing");
        File::create(&wrong).expect("failed to create tempfile");
        symlink::symlink_file(&wrong, &ln.head).expect("failed to create symlink");
        assert!(matches!(ln.status(), LinkStatus::Invalid(_)));
        symlink::remove_symlink_file(&ln.head).expect("failed to remove symlink");
        // Head is a file
        File::create(&ln.head).expect("failed to create tempfile");
        assert!(matches!(ln.status(), LinkStatus::Invalid(_)));
    }

    #[test]
    fn link_normal() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let ln = Link::new(dir.path().join("link.head"), dir.path().join("link.tail"));
        File::create(&ln.tail).expect("failed to create tempfile");
        // Link once
        ln.link().expect("failed to create link");
        assert_eq!(ln.head.read_link().expect("failed to read link"), ln.tail);
    }

    #[test]
    fn unlink_normal() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let ln = Link::new(dir.path().join("link.head"), dir.path().join("link.tail"));
        File::create(&ln.tail).expect("failed to create tempfile");
        // Link and unlink once
        ln.link().expect("failed to create link");
        ln.unlink().expect("failed to remove link");
        assert!(!ln.head.exists());
    }
}
