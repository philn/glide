project(
  'glide', 'rust',
  version: '0.5.9',
  license: 'MIT',
)

glide_version = meson.project_version()
version_array = glide_version.split('.')
glide_major_version = version_array[0].to_int()
glide_minor_version = version_array[1].to_int()
glide_version_micro = version_array[2].to_int()

datadir = get_option('datadir')
icondir = join_paths(datadir, 'icons')

install_data('data/net.baseart.Glide.desktop', install_dir : datadir + '/applications')
install_data('data/net.baseart.Glide.svg', install_dir : icondir + '/hicolor/scalable/apps/')
install_data('data/net.baseart.Glide.metainfo.xml', install_dir : datadir + '/metainfo/')

meson.add_dist_script(
  'build-aux/dist-vendor.sh',
  meson.project_build_root() / 'meson-dist' / meson.project_name() + '-' + meson.project_version(),
  meson.project_source_root()
)

meson.add_install_script('scripts/meson_post_install.py')
