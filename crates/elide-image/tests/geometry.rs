//! The image geometry primitives compose as expected.

use elide_image::primitive::{BoundingBox, Point, Polygon};

#[test]
fn geometry_shapes_compose() {
    let bbox = BoundingBox::from_origin_size(Point::new(10.0, 20.0), 100.0, 40.0);
    assert_eq!(bbox.width(), 100.0);
    assert_eq!(bbox.height(), 40.0);
    assert_eq!(bbox.max, Point::new(110.0, 60.0));

    let poly: Polygon = [
        Point::new(0.0, 0.0),
        Point::new(1.0, 0.0),
        Point::new(0.0, 1.0),
    ]
    .into_iter()
    .collect();
    assert_eq!(poly.len(), 3);
}
