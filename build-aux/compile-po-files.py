
import os
import sys

def main(args):
    prefix = args[0]
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    po_dir = os.path.join(root, 'po')
    for filename in os.listdir(po_dir):
        if not filename.endswith('.po'):
            continue
        basename, _ = os.path.splitext(filename)
        po_file = os.path.join(po_dir, filename)
        output_dir = os.path.join(prefix, basename, "LC_MESSAGES")
        try:
            os.makedirs(output_dir)
        except FileExistsError:
            pass
        output = os.path.join(output_dir, "glide.mo")
        os.system(f"msgfmt -o {output} {po_file}")

    return 0

if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
