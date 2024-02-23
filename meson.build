project(
  'glide', 'rust',
  version: '0.6.1',
  license: 'MIT',
)

glide_version = meson.project_version()
version_array = glide_version.split('.')
glide_major_version = version_array[0].to_int()
glide_minor_version = version_array[1].to_int()
glide_version_micro = version_array[2].to_int()

datadir = get_option('datadir')
icondir = join_paths(datadir, 'icons')

app_id = 'net.baseart.Glide'
icon_name = '@0@.svg'.format(app_id)

if get_option('profile') == 'development'
  profile = 'Devel'
  install_data('data/net.baseart.Glide.Devel.svg', install_dir : icondir + '/hicolor/scalable/apps/',
               rename: '@0@.svg'.format(app_id))
else
  install_data('data/net.baseart.Glide.svg', install_dir : icondir + '/hicolor/scalable/apps/')
endif

install_data('data/net.baseart.Glide.desktop', install_dir : datadir + '/applications')
install_data('data/net.baseart.Glide.metainfo.xml', install_dir : datadir + '/metainfo/')

meson.add_dist_script(
  'build-aux/dist-vendor.sh',
  meson.project_build_root() / 'meson-dist' / meson.project_name() + '-' + meson.project_version(),
  meson.project_source_root()
)

meson.add_install_script('scripts/meson_post_install.py')

cargo_build = custom_target(
  'cargo-build',
  build_by_default: true,
  output: [meson.project_name()],
  console: true,
  install: true,
  install_dir: get_option('bindir'),
  command: [
    find_program(meson.project_source_root() / 'build-aux' / 'cargo-build.py'),
    meson.project_source_root(), meson.project_build_root(), meson.project_name(), '@OUTDIR@'
  ]
)
