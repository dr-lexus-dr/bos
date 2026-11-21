// bOS's viewport and input adapter for go-webengine v0.48.0.
// The renderer preserves its live JavaScript session and form controls on resize.
package engine

import (
	"context"
	"image"
	"strings"

	"github.com/go-webengine/engine/dom"
	"github.com/go-webengine/engine/js"
	"github.com/go-webengine/engine/layout"
	"github.com/go-webengine/engine/paint"
)

func (d *LiveDocument) Resize(ctx context.Context, size image.Rectangle) (*image.RGBA, *RenderInfo, error) {
	if d.closed {
		return nil, nil, ErrClosed
	}
	d.viewport = size
	d.vpW, d.vpH = viewportSize(size)
	d.rp.sheets = d.e.fetchExternalSheets(ctx, d.doc, float64(d.vpW))
	return d.Interact(ctx, func() {})
}

// The painter uses a zero-origin surface. Translate the layout only while
// painting, so DOM hit tests and focus coordinates stay in document space.
func (d *LiveDocument) desktopFrame() *image.RGBA {
	d.desktopScroll = min(max(0, d.desktopScroll), max(0, int(d.rp.height)-d.vpH))
	clipped := *d.rp
	clipped.height = float64(d.vpH)
	img := d.e.newCanvas(d.doc, &clipped, d.viewport, d.vpW)
	layout.PaintAtScroll(d.rp.box, d.desktopScroll, func() {
		paint.PaintFull(img, d.rp.box, d.fonts, d.rp.imgs, d.rp.bgImgs)
	})
	return img
}

func (d *LiveDocument) Scroll(y int) (*image.RGBA, *RenderInfo, error) {
	d.desktopScroll = y
	return d.Frame()
}

func (d *LiveDocument) ScrollOffset() int { return d.desktopScroll }

func (d *LiveDocument) BeginInput() { d.sess.BeginInput() }

// ChangeControl applies the default user action after a click or selection key.
// The engine has already dispatched the cancelable click to page listeners.
func (d *LiveDocument) ChangeControl(ctx context.Context, n *dom.Node, direction int) (*image.RGBA, *RenderInfo, error) {
	return d.Interact(ctx, func() {
		set := func(el *dom.Node, key string) {
			if el.Attr == nil {
				el.Attr = make(map[string]string)
			}
			el.Attr[key] = ""
		}
		if n.Tag == "select" {
			var options []*dom.Node
			index := 0
			for _, option := range d.QuerySelectorAll("option") {
				parent := option.Parent
				for parent != nil && parent.Tag != "select" {
					parent = parent.Parent
				}
				if parent != n {
					continue
				}
				if _, disabled := option.Attribute("disabled"); disabled {
					continue
				}
				if _, selected := option.Attribute("selected"); selected {
					index = len(options)
				}
				options = append(options, option)
			}
			if len(options) < 2 {
				return
			}
			for _, option := range options {
				delete(option.Attr, "selected")
			}
			set(options[(index+direction+len(options))%len(options)], "selected")
		} else if strings.EqualFold(n.Attr["type"], "radio") {
			if _, checked := n.Attribute("checked"); checked {
				return
			}
			form := func(el *dom.Node) *dom.Node {
				for ; el != nil && el.Tag != "form"; el = el.Parent {
				}
				return el
			}
			for _, other := range d.QuerySelectorAll("input[type=radio]") {
				if n.Attr["name"] != "" && other.Attr["name"] == n.Attr["name"] && form(other) == form(n) {
					delete(other.Attr, "checked")
				}
			}
			set(n, "checked")
		} else if _, checked := n.Attribute("checked"); checked {
			delete(n.Attr, "checked")
		} else {
			set(n, "checked")
		}
		for _, name := range []string{"input", "change"} {
			d.sess.Dispatch(n, name, js.EventInit{Bubbles: true})
		}
	})
}

// OpenDesktopPage publishes the first styled viewport while the same document
// continues to load images and scripts. Interaction uses the resulting session.
func (e *Engine) OpenDesktopPage(ctx context.Context, doc *Document, viewport image.Rectangle, initial func(*image.RGBA)) *LiveDocument {
	fonts := paint.NewFonts()
	w, h := viewportSize(viewport)
	rp, sess, stop := e.renderCoreStaged(ctx, doc, w, h, fonts, func(stage string, p *renderPass) {
		if stage != "initial" {
			return
		}
		clipped := *p
		clipped.height = float64(h)
		img := e.newCanvas(doc, &clipped, viewport, w)
		paint.PaintFull(img, p.box, fonts, p.imgs, p.bgImgs)
		initial(img)
	})
	return &LiveDocument{e: e, doc: doc, rp: rp, sess: sess, fonts: fonts, viewport: viewport, vpW: w, vpH: h, stopHeap: stop, prevLinks: linkKey(doc.Root)}
}
