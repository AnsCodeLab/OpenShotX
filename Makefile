# OpenShotX — build & install
#
#   make            build the release binary
#   make install    install to ~/.local (PREFIX=/usr/local make install for system)
#   make uninstall  remove installed files
#   make dist       build a release tarball under dist/
#   make clean      cargo clean + remove dist/

PREFIX ?= $(HOME)/.local
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH := $(shell uname -m)
DISTNAME := openshotx-$(VERSION)-$(ARCH)-linux
RPMTOP := $(CURDIR)/dist/rpmbuild
SRCNAME := openshotx-$(VERSION)

.PHONY: build install uninstall dist rpm clean

build:
	cargo build --release

install: build
	PREFIX="$(PREFIX)" ./install.sh --no-build

uninstall:
	PREFIX="$(PREFIX)" ./uninstall.sh

# Self-contained tarball: binary + assets + installer + docs.
dist: build
	@rm -rf dist/$(DISTNAME)
	@mkdir -p dist/$(DISTNAME)/data
	cp target/release/openshotx        dist/$(DISTNAME)/
	cp data/openshotx.svg              dist/$(DISTNAME)/data/
	cp data/openshotx.desktop          dist/$(DISTNAME)/data/
	cp data/io.github.anscodelab.openshotx.metainfo.xml dist/$(DISTNAME)/data/
	cp install.sh uninstall.sh README.md dist/$(DISTNAME)/
	tar -C dist -czf dist/$(DISTNAME).tar.gz $(DISTNAME)
	@echo "==> dist/$(DISTNAME).tar.gz"

# Build a binary RPM (Fedora). Packages the pre-built binary + assets; gives a
# native install/uninstall via dnf or GNOME Software.
rpm: build
	rm -rf $(RPMTOP) dist/$(SRCNAME)
	mkdir -p $(RPMTOP)/SOURCES $(RPMTOP)/SPECS $(RPMTOP)/BUILD
	mkdir -p dist/$(SRCNAME)/data
	cp target/release/openshotx dist/$(SRCNAME)/
	cp data/openshotx.svg data/openshotx.desktop \
	   data/io.github.anscodelab.openshotx.metainfo.xml dist/$(SRCNAME)/data/
	tar -C dist -czf $(RPMTOP)/SOURCES/$(SRCNAME).tar.gz $(SRCNAME)
	cp packaging/openshotx.spec $(RPMTOP)/SPECS/
	rpmbuild --define "_topdir $(RPMTOP)" --define "version $(VERSION)" \
	         -bb $(RPMTOP)/SPECS/openshotx.spec
	@echo "==> RPM(s):"; find $(RPMTOP)/RPMS -name '*.rpm'

clean:
	cargo clean
	rm -rf dist
