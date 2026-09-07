#!/bin/sh

set -e

# The documentation reads the public managed surface, and this environment has no Rust toolchain:
# the binding's build shells out to cargo and stops the build when it fails. Its assembly is pure
# managed, so skipping the engine costs the documentation nothing.
SkipNativeEngine=true
export SkipNativeEngine

dotnet tool install -g docfx
dotnet build packages/csharp/ArmoniK.Api.sln
docfx docfx.json
sed -E -i 's/([#]+) <a id="([^"]+)"><\/a> (.+)/<a id="\2"><\/a>\n\1 \3/g' .docs/content/api/csharp/*.md
