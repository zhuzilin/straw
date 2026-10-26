//! Resolve one path component using only filesystem and Path primitives.

pub trait Model: crate::namespace_target::Model {
    type Error;
}

pub trait Primitives: Model + crate::namespace_target::Backend {
    fn root(&self, prefix: &str) -> Self::Path;

    fn cached(&self, prefix: &str, identity: &str) -> Self::Path;

    fn child(&self, base: &Self::Path, component: &str, prefix: &str) -> Self::Path;
    // `pending` binds the normalized parent, its child and the original name.
    // This is the filesystem's stable namespace law for this readlink call.

    fn child_link(
        &mut self,
        base: &Self::Path,
        candidate: &Self::Path,
        prefix: &str,
    ) -> Result<Self::Path, Self::IoError>;

    fn join(&self, base: &Self::Path, target: Self::Path) -> Self::Path;

    fn relative(&self, path: &Self::Path, prefix: &str) -> Result<String, Self::Error>;
    fn io_error(&self, error: Self::IoError) -> Self::Error;
    fn target_error(&self, error: crate::namespace_target::Error<Self::IoError>) -> Self::Error;
}

pub struct Backend<P: Primitives> {
    primitives: P,
}

impl<P: Primitives> crate::namespace_identity::BackendModel for Backend<P> {
    type Position = P::Path;
}

impl<P: Primitives> Backend<P> {
    pub fn new(primitives: P) -> Self {
        Self { primitives }
    }
}

impl<P: Primitives> crate::namespace_identity::Backend for Backend<P> {
    type Error = P::Error;
    fn root(&self, prefix: &str) -> Self::Position {
        self.primitives.root(prefix)
    }
    fn cached_position(&self, prefix: &str, identity: &str) -> Self::Position {
        self.primitives.cached(prefix, identity)
    }
    fn step(
        &mut self,
        position: Self::Position,
        component: &str,
        prefix: &str,
    ) -> Result<(Self::Position, String), Self::Error> {
        let candidate = self.primitives.child(&position, component, prefix);
        let resolved = match self.primitives.child_link(&position, &candidate, prefix) {
            Ok(target) => {
                let path = if self.primitives.is_absolute(&target) {
                    target
                } else {
                    self.primitives.join(&position, target)
                };
                match crate::namespace_target::resolve(&mut self.primitives, path) {
                    Ok(resolved) => resolved,
                    Err(error) => return Err(self.primitives.target_error(error)),
                }
            }
            Err(error) => {
                if matches!(
                    self.primitives.classify(&error),
                    crate::namespace_target::ErrorKind::Missing
                        | crate::namespace_target::ErrorKind::NotLink
                ) {
                    candidate
                } else {
                    return Err(self.primitives.io_error(error));
                }
            }
        };
        let identity = self.primitives.relative(&resolved, prefix)?;
        Ok((resolved, identity))
    }
}
