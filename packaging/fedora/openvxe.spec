# Built offline from two sources made by make-sources.sh: the source tree
# (git archive) and the vendored crates (cargo vendor), so it also builds in
# mock and COPR without network access.
%bcond check 1

# make-sources.sh defines gitrev as the short hash of the packaged commit.
Name:           openvxe
Version:        0.1.0
Release:        1%{?gitrev:.git%{gitrev}}%{?dist}
Summary:        Configuration TUI and battery tray icon for the VXE R1 SE+ wireless mouse

License:        GPL-3.0-only
URL:            https://github.com/FrancescoLFM/OpenVXE
Source0:        %{name}-%{version}.tar.gz
Source1:        %{name}-%{version}-vendor.tar.xz

ExclusiveArch:  %{rust_arches}

BuildRequires:  cargo
BuildRequires:  rust >= 1.88
BuildRequires:  gcc
BuildRequires:  pkgconfig(libudev)
BuildRequires:  desktop-file-utils
BuildRequires:  systemd-rpm-macros
BuildRequires:  rust-packaging

Requires:       systemd-udev
Suggests:       konsole

%description
OpenVXE is an open-source configuration tool for the VXE R1 SE+ wireless
mouse. It ships the openvxe TUI (DPI, polling rate, buttons and lighting),
the openvxe-tray system tray icon with the battery level, the vxe-probe and
vxe-decode reverse-engineering helpers, and udev rules that let the
logged-in user talk to the mouse without root.

%prep
%autosetup -n %{name}-%{version} -a1
mkdir -p .cargo
cat > .cargo/config.toml <<'CFG'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
CFG

%build
%set_build_flags
export RUSTFLAGS="%{build_rustflags}"
export CARGO_TARGET_DIR=target
cargo build --offline --locked --release --bins

%install
install -Dm755 -t %{buildroot}%{_bindir} \
    target/release/openvxe \
    target/release/vxe-probe \
    target/release/vxe-decode \
    target/release/openvxe-tray
install -Dm644 udev/70-openvxe.rules %{buildroot}%{_udevrulesdir}/70-openvxe.rules
desktop-file-install --dir=%{buildroot}%{_datadir}/applications \
    packaging/desktop/openvxe-tray.desktop
install -Dm644 packaging/desktop/openvxe-tray.desktop \
    %{buildroot}%{_sysconfdir}/xdg/autostart/openvxe-tray.desktop

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/openvxe-tray.desktop
%if %{with check}
export RUSTFLAGS="%{build_rustflags}"
export CARGO_TARGET_DIR=target
cargo test --offline --locked --release
%endif

# systemd-udev reloads the rules through its file triggers.
%post
echo ">>> udev rules installed: unplug and replug the VXE USB dongle"
echo ">>> to use openvxe without root."

%files
%license LICENSE
%doc README.md docs/PROTOCOL.md docs/CAPTURE.md
%{_bindir}/openvxe
%{_bindir}/openvxe-tray
%{_bindir}/vxe-probe
%{_bindir}/vxe-decode
%{_udevrulesdir}/70-openvxe.rules
%{_datadir}/applications/openvxe-tray.desktop
%config(noreplace) %{_sysconfdir}/xdg/autostart/openvxe-tray.desktop

%changelog
* Fri Oct 16 2026 FrancescoLFM <speedartemmekappose@gmail.com> - 0.1.0-1
- Initial Fedora package
