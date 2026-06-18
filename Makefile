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

.PHONY: build install uninstall dist clean

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
	cp install.sh uninstall.sh README.md dist/$(DISTNAME)/
	tar -C dist -czf dist/$(DISTNAME).tar.gz $(DISTNAME)
	@echo "==> dist/$(DISTNAME).tar.gz"

clean:
	cargo clean
	rm -rf dist
