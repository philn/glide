project(
  'glide', 'rust',
  version: '0.5.7',
  license: 'MIT',
)

glide_version = meson.project_version()
version_array = glide_version.split('.')
glide_major_version = version_array[0].to_int()
glide_minor_version = version_array[1].to_int()
glide_version_micro = version_array[2].to_int()

glide_prefix = get_option('prefix')
glide_bindir = join_paths(glide_prefix, get_option('bindir'))
glide_localedir = join_paths(glide_prefix, get_option('localedir'))

datadir = get_option('datadir')
icondir = join_paths(datadir, 'icons')

cargo = find_program('cargo')
cargo_vendor = find_program('cargo-vendor', required: false)
cargo_script = find_program('scripts/cargo.sh')
grabber = find_program('scripts/grabber.sh')
cargo_release = find_program('scripts/release.sh')

c = run_command(grabber)
sources = c.stdout().strip().split('\n')

install_data('data/net.baseart.Glide.desktop', install_dir : datadir + '/applications')
install_data('data/net.baseart.Glide.svg', install_dir : icondir + '/hicolor/scalable/apps/')
install_data('data/net.baseart.Glide.appdata.xml', install_dir : datadir + '/appdata/')

cargo_release = custom_target('cargo-build',
                        build_by_default: true,
                        console: true,
                        input: sources,
                        output: ['glide'],
                        install: true,
                        install_dir: glide_bindir,
                        command: [cargo_script, '@CURRENT_SOURCE_DIR@', '@OUTPUT@', glide_localedir])

run_target('release', command: ['scripts/release.sh',
                                meson.project_name() + '-' + glide_version
                               ])

meson.add_install_script('scripts/meson_post_install.py')
