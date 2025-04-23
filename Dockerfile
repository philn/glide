FROM fedora:42

RUN dnf update -y
RUN dnf install -y gcc git wget gtk4-devel gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-bad-free-devel libadwaita-devel

RUN wget -O- https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
