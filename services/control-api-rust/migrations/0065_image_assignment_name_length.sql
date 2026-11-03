-- 0065_image_assignment_name_length - PostgreSQL regular expressions allow a repetition count of
-- 255 at most, so the 300-character pattern of 0064 rejected every insert ("invalid repetition
-- count"). An image reference is at most 255 characters here.

ALTER TABLE image_assignments DROP CONSTRAINT image_assignments_image_shape;
ALTER TABLE image_assignments ADD CONSTRAINT image_assignments_image_shape
    CHECK (char_length(image) <= 255 AND image ~ '^[A-Za-z0-9][A-Za-z0-9._/:@-]*$');
