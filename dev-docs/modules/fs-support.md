# Filesystem support

[Implementation](../../src/fs_support/)

Atomic writes use a temporary file in the destination directory, fsync the
contents, rename, and attempt to flush the parent. SHA-256 helpers verify the
exact bytes of truth files, operation backups and bundle payloads. Existence
markers distinguish a copied file from a file that was absent before backup.

The shared tree copier supports two policies. Bundle snapshots reject links
and skip caller-selected names such as `.git`. Node backups copy links without
following their targets. On Windows the reparse metadata provides the link's
own type even when the target is missing. Junctions retain their mount-point
reparse data when copied. Unsupported reparse types fail before traversal.

Tree removal unlinks Linux symlinks directly and uses the link's own directory
attribute on Windows. Clear read-only attributes only on ordinary Windows
files. Shared move helpers rename on the same volume and check destination
absence. Bundle import records each move in its operation journal before
renaming so that rollback can infer whether the move took effect.

Temporary staging directories are RAII-managed until a transaction or operation
records their location and takes over cleanup.
