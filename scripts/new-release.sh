#!/bin/bash


current=$(cat meson.build | grep -Eo "version: '(.*)'" | grep -Eo "[0-9]+\.[0-9]+.[0-9]+")
mayor=$(echo $current | cut -d"." -f1)
minor=$(echo $current | cut -d"." -f2)
rev=$(echo $current | cut -d"." -f3)

case $1 in
"mayor")
    next=$(echo $(($mayor + 1)).0.0)
    ;;
"minor")
    next=$(echo $mayor.$(($minor + 1)).0)
    ;;
*)
    next=$(echo $mayor.$minor.$(($rev + 1)))
    ;;
esac

sed -i "s/$current/$next/" meson.build
sed -i "s/$current/$next/" Cargo.toml

cargo check

git commit -av
git tag -s $next

ninja -C _build release