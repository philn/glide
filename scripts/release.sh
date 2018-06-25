#!/bin/sh

VERSION=$1
DEST=${MESON_BUILD_ROOT}
DIST=$DEST/dist/$VERSION


cd "${MESON_SOURCE_ROOT}"
mkdir -p $DIST

# copying files
rsync -rR $(git ls-files) $DIST

# cargo vendor
mkdir $DIST/.cargo
cargo vendor | sed 's/^directory = ".*"/directory = "vendor"/g' > $DIST/.cargo/config
cp -rf vendor $DIST/

# packaging
cd $DEST/dist
tar -cJvf $VERSION.tar.xz $VERSION
