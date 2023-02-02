Glide Media Player
==================

Glide is a simple and minimalistic media player relying on
[GStreamer](http://gstreamer.freedesktop.org) for the multimedia support
and [GTK+](http://gtk.org) for the user interface. Glide should be able
to play any multimedia format supported by
[GStreamer](http://gstreamer.freedesktop.org), locally or remotely
hosted. Glide is developed in [Rust](http://rust-lang.org) and was
tested on Linux and macOS so far. It should also work on Windows, please
let me know if anyone managed to test it on that platform.

I aim to keep this project simple and it probably won't grow to become a
very complicated GUI. If you feel adventurous and willing to help, feel
free to pick up a task from the TODO list and open a PR. Users are also
encouraged to file issues on the Github bug tracker of course.

![alt text](https://github.com/philn/glide/raw/master/screenshot.png "Glide screenshot")
![alt text](https://github.com/philn/glide/raw/master/audio-screenshot.png "Glide audio playback screenshot")

Installation
------------

Install it with Cargo:

1.  Install [RustUp](https://rustup.rs):

        curl https://sh.rustup.rs -sSf | sh

2.  Install GStreamer and GTK+. On Debian/Linux:

        sudo apt install gstreamer1.0-plugins-{base,good,bad} libgstreamer-plugins-{bad,base}1.0-dev
        sudo apt install libgtk-3-dev gstreamer1.0-{gl,gtk3}

    On macOS, with [brew](http://brew.sh):

        brew install pango gstreamer gtk+3
        brew install --build-from-source --with-pango --with-{libogg,libvorbis,opus,theora} gst-plugins-base
        brew install --build-from-source --with-libvpx gst-plugins-good
        brew install gst-plugins-bad

3.  Install Glide:

        cargo install glide
        # or if you want to have automatic update checking:
        cargo install --features self-updater glide

### Packaging status

#### Flatpak

This is the most recommended way to use Glide as it will allow the maintainers to more
easily reproduce reported bugs.

Glide is available on [Flathub](https://flathub.org/apps/details/net.baseart.Glide).
After setting up the flathub Flatpak remote as documented in Flathub, install with the following command, or
through GNOME Software.

    flatpak install net.baseart.Glide

#### Fedora

Available in [COPR](https://copr.fedorainfracloud.org/coprs/atim/glide-rs/):

    sudo dnf copr enable atim/glide-rs -y
    sudo dnf install glide-rs

Using Glide
-----------

There is currently only one way to use Glide, using the command line
interface:

    glide /path/to/localfile.mp4 http://some.com/remote/file.mp4

At some point I will add file chooser support and improve integration
for desktop so that all you need to do is double-click on a media file
or drag it to the Glide window.

Once running you can use some menus to switch the subtitle and audio
tracks, play, pause, seek and switch the window to fullscreen. There are
also some keyboard shortcuts for these actions:

- play/pause: space
- seek forward: meta-right or alt-left
- seek backward: meta-left or alt-right
- switch to fullscreen: meta-f or alt-f
- exit from fullscreen: escape
- quit the application: meta-q or ctrl-q
- load a subtitle file: meta-s or alt-s
- increase volume: meta-up or alt-up
- decrease volume: meta-up or alt-down
- mute the audio track: meta-m or alt-m
- open a new file: meta-o or alt-o

Contact
-------

I usually hang out on Freenode IRC, in \#gstreamer using the philn
nickname. Feel free to also reach out by mail (check git logs to find my
address).

Release procedure
-----------------

- Bump version in `Cargo.toml` and `meson.build`
- Add release info in appstream file and make sure it is valid...

      appstream-util validate data/net.baseart.Glide.metainfo.xml

- Commit and tag new version:

      git ci -am "Bump to ..."
      git tag -s "version..."

- Build tarball:

      cargo install cargo-vendor
      pip3 install --user -U meson
      meson setup _build
      meson dist -C _build

- Publish version and tag:

      git push --tags
      git push

- Update crate on crates.io:

      cargo package
      cargo publish

- Upload tarball from `_build/meson-dist/` to Github
- TODO: Upload self-update binaries to Github
