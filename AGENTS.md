# Project maintenance

Maintain release notes for every major and minor release, including pre-1.0
minor releases. Follow `docs/RELEASE.md` and `docs/releases/README.md`.

Keep user-visible changes in `CHANGELOG.md` under Unreleased as work lands.
Before a versioned release, create `docs/releases/vX.Y.Z.md` using
`docs/releases/TEMPLATE.md`, update the release index, and move the relevant
changelog entries into a dated version section. Include compatibility and
migration guidance, actual validation evidence, and known limitations.

Preserve historical release notes. Add clearly dated corrections or follow-up
notes rather than rewriting past claims. Never describe an unrun check as
passed. Documentation-only maintenance does not itself require a version bump.
