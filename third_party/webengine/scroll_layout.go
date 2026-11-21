// bOS paints a viewport-sized surface while keeping the document hit map intact.
package layout

func PaintAtScroll(box *Box, y int, paint func()) {
	if y != 0 {
		dy := float64(y)
		translateBox(box, 0, -dy)
		defer translateBox(box, 0, dy)
	}
	paint()
}
