#!/usr/bin/env python3

import argparse
import os
import sys
import subprocess
import shutil

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('srcdir')
    parser.add_argument('builddir')
    parser.add_argument('project_name')
    parser.add_argument('outdir')
    args = parser.parse_args()

    cmd = ['cargo', 'build', '--manifest-path', os.path.join(args.srcdir, 'Cargo.toml'),
           '--target-dir', args.builddir, '--release', '--features', 'wayland,x11egl,x11glx']
    subprocess.run(cmd)
    shutil.copy2(os.path.join(args.builddir, 'release', args.project_name), os.path.join(args.outdir, args.project_name))
    return 0


if __name__ == "__main__":
    sys.exit(main())
