# Glide Media Player

Glide is a simple and minimalistic media player relying on
[GStreamer](http://gstreamer.freedesktop.org) for the multimedia support
and [GTK](http://gtk.org) for the user interface. Glide should be able
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

## Installation

Install it with Cargo:

1.  Install [RustUp](https://rustup.rs):

        curl https://sh.rustup.rs -sSf | sh

2.  Install GStreamer and GTK+. On Debian/Linux:

        sudo apt install gstreamer1.0-plugins-{base,good,bad} libgstreamer-plugins-{bad,base}1.0-dev
        sudo apt install libgtk-4-dev gstreamer1.0-gl libadwaita-1-dev

    On macOS, with [brew](http://brew.sh):

        brew install pango gstreamer gtk+4 libadwaita
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

## Using Glide

When used from the installed Flatpak, Glide can be set up as default media
player, so double-clicking on a media file in your favorite file browser should
bring up Glide.

Glide can also be used from the command line interface. In a terminal:

```bash
$ # starting the flatpak version
$ flatpak run net.baseart.Glide /path/to/localfile.mp4 http://some.com/remote/file.mp4
$ # starting the version installed with cargo or traditional distro packages
$ glide /path/to/localfile.mp4 http://some.com/remote/file.mp4
```

Once running you can use some menus to switch the subtitle and audio
tracks, play, pause, seek and switch the window to fullscreen. There are
also some keyboard shortcuts for these actions:

- show shortcuts window: meta-? or ctrl-?
- play/pause: space
- seek forward: meta-right or ctrl-right
- seek backward: meta-left or ctrl-left
- switch to fullscreen: meta-f or ctrl-f
- exit from fullscreen: escape
- quit the application: meta-q or ctrl-q
- load a subtitle file: meta-s or ctrl-s
- increase volume: meta-up or ctrl-up
- decrease volume: meta-up or ctrl-down
- mute the audio track: meta-m or ctrl-m
- open a new file: meta-o or ctrl-o

## Contacting the maintainer

Philippe usually hangs out on Freenode IRC, in \#gstreamer using the philn
nickname. Feel free to also reach out by mail (check git logs to find the
address).
