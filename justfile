metainfo := "data/net.baseart.Glide.metainfo.xml"

manifest-check:
    flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest build-aux/net.baseart.Glide.Devel.json

metainfo-check:
    appstreamcli validate {{metainfo}}
    flatpak run --command=flatpak-builder-lint org.flatpak.Builder appstream {{metainfo}}

[confirm("Have you added the new release notes in data/net.baseart.Glide.metainfo.xml?")]
release version: metainfo-check
    meson rewrite kwargs set project / version {{version}}
    cargo set-version {{version}}
    git commit -am "Bump to {{version}}"
    git tag -s {{version}}
    meson setup _build
    meson dist -C _build
    mv _build/meson-dist/* /tmp
    @echo "Now pending, upload /tmp/glide-{{version}}.tar.xz to GitHub"

publish:
    git push --tags
    git push
    cargo package
    cargo publish
