-- Make message_id nullable so attachments can be uploaded before message is created
ALTER TABLE message_attachments
    ALTER COLUMN message_id DROP NOT NULL;

-- Drop the existing foreign key constraint
ALTER TABLE message_attachments
    DROP CONSTRAINT message_attachments_message_id_fkey;

-- Re-add the foreign key constraint without NOT NULL
ALTER TABLE message_attachments
    ADD CONSTRAINT message_attachments_message_id_fkey
    FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE;
