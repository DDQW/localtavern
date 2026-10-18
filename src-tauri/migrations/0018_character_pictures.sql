-- Picture generation per character: the look/seed every picture shares, costumes with their own
-- expression packs, and labelled action pictures.
ALTER TABLE Characters ADD COLUMN PictureStyleJson TEXT NULL;
ALTER TABLE Characters ADD COLUMN OutfitsJson TEXT NOT NULL DEFAULT '[]';
ALTER TABLE Characters ADD COLUMN ActionPicturesJson TEXT NOT NULL DEFAULT '[]';
