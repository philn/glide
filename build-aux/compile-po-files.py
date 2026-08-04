
import os
import sys

def main(*args):
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    po_dir = os.path.join(root, 'po')
    for filename in os.listdir(po_dir):
        if not filename.endswith('.po'):
            continue
        basename, _ = os.path.splitext(filename)
        po_file = os.path.join(po_dir, filename)
        output = os.path.join(po_dir, f"{basename}.mo")
        os.system(f"msgfmt -o {output} {po_file}")

    return 0

if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
