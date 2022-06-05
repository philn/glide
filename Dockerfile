FROM fedora:latest

RUN dnf update -y
RUN dnf install -y git wget gtk4-devel gstreamer1-devel gstreamer1-plugins-base-devel
