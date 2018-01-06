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
encourage to file issues on the Github bug tracker of course.

Installation
------------

Currently Glide isn't packaged for any Linux distribution or macOS
package manager. So the only way to install it is with Cargo:

1.  Install [RustUp](https://rustup.rs): :

        $ curl https://sh.rustup.rs -sSf | sh

2.  Install GStreamer and GTK+. On Debian/Linux: :

        $ sudo apt install gstreamer1.0-plugins-{base,good,bad} libgstreamer-plugins-{bad,base}1.0-dev
        $ sudo apt install libgtk-3-dev

    On macOS, with [brew](http://brew.sh): :

        $ brew install gst-plugins-{base,good,bad} gstreamer gtk+3

3.  Install Glide:

        cargo install glide

Using Glide
-----------

There is currently only one way to use Glide, using the command line
interface:

    $ glide /path/to/localfile.mp4 http://some.com/remote/file.mp4

At some point I will add file chooser support and improve integration
for desktop so that all you need to do is double-click on a media file
or drag it to the Glide window.

Once running you can use some menus to switch the subtitle and audio
tracks, play, pause, seek and switch the window to fullscreen. There are
also some keyboard shortcuts for these actions:

-   play/pause: space
-   seek forward: meta-right or ctrl-left
-   seek backward: meta-left or ctrl-right
-   switch to fullscreen: meta-f or ctrl-f
-   exit from fullscreen: escape
-   quit the application: meta-q or ctrl-q

Contact
-------

I usually hang out on Freenode IRC, in \#gstreamer using the philn
nickname. Feel free to also reach out by mail (check git logs to find my
address).
