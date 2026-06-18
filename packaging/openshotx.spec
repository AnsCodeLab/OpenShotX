# Packages the pre-built openshotx binary (built by `make rpm`, not inside rpmbuild)
# along with the icon, desktop entry, and AppStream metainfo.
# Pre-built binary: no sources to build, so skip the debuginfo/debugsource subpackages.
%global debug_package %{nil}

Name:           openshotx
Version:        %{version}
Release:        1%{?dist}
Summary:        Screenshot and screen-recording tool for Linux (X11 and Wayland)

License:        WTFPL
URL:            https://github.com/AnsCodeLab/OpenShotX
Source0:        %{name}-%{version}.tar.gz
ExclusiveArch:  x86_64

# Shared libraries the binary links against at runtime.
Requires:       gtk4
Requires:       libadwaita
Requires:       gstreamer1
Requires:       gstreamer1-plugins-base
Requires:       tesseract
Requires:       leptonica
# Used by sub-features (clipboard, recording); not hard requirements.
Recommends:     gstreamer1-plugins-good
Recommends:     ffmpeg-free
Recommends:     wl-clipboard
Recommends:     xclip
Recommends:     tesseract-langpack-eng

%description
OpenShotX is a native screenshot and screen-recording tool for Linux (X11 and
Wayland), inspired by CleanShot X. It supports area, screen, and window capture,
screen recording to MP4 or GIF, OCR text extraction, and a system-tray
quick-capture menu.

%prep
%setup -q

%install
install -Dm755 openshotx %{buildroot}%{_bindir}/openshotx
install -Dm644 data/openshotx.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/openshotx.svg
install -Dm644 data/openshotx.desktop \
    %{buildroot}%{_datadir}/applications/openshotx.desktop
install -Dm644 data/io.github.anscodelab.openshotx.metainfo.xml \
    %{buildroot}%{_datadir}/metainfo/io.github.anscodelab.openshotx.metainfo.xml

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/openshotx.desktop || :

%files
%{_bindir}/openshotx
%{_datadir}/icons/hicolor/scalable/apps/openshotx.svg
%{_datadir}/applications/openshotx.desktop
%{_datadir}/metainfo/io.github.anscodelab.openshotx.metainfo.xml

%changelog
* Thu Jun 18 2026 AnsCodeLab <annguyen209@gmail.com> - 0.1.0-1
- Initial RPM package (pre-built binary).
