-- The client half of the kit's items: their names, descriptions and looks.
-- llchrisll's English translation (ROenglishRE), which the players install,
-- loads this file from SystemEN/itemInfo.lua and adds tbl_custom's items to
-- its own list, so this file holds only the server's items.
--
-- Each key is the item's Id in db/item_db.yml. An Id the official list
-- already has is skipped here; to change an official item, put it in
-- tbl_override instead.
--
-- identifiedResourceName names the sprite and inventory icon. These reuse the
-- Red Potion's and the Ribbon's, whose names are Korean: written here as
-- byte escapes (\187\161...), since the client reads them in the Korean code
-- page. ClassNum is the look worn on the head, the same number as View in
-- db/item_db.yml.
tbl_custom = {
	[40000] = {
		unidentifiedDisplayName = "Kit Potion",
		unidentifiedResourceName = "\187\161\176\163\198\247\188\199",
		unidentifiedDescriptionName = { "" },
		identifiedDisplayName = "Kit Potion",
		identifiedResourceName = "\187\161\176\163\198\247\188\199",
		identifiedDescriptionName = {
			"A potion from the item kit.",
			"Restores ^0000FF100^000000 HP.",
			"________________________",
			"Type:^777777 Restorative^000000",
			"Weight:^777777 7^000000"
		},
		slotCount = 0,
		ClassNum = 0,
		costume = false
	},
	[40001] = {
		unidentifiedDisplayName = "Ribbon",
		unidentifiedResourceName = "\184\174\186\187",
		unidentifiedDescriptionName = { "" },
		identifiedDisplayName = "Kit Ribbon",
		identifiedResourceName = "\184\174\186\187",
		identifiedDescriptionName = {
			"A ribbon from the item kit.",
			"MDEF +5",
			"________________________",
			"Type:^777777 Headgear^000000",
			"Defense:^777777 1^000000",
			"Position:^777777 Upper^000000",
			"Weight:^777777 10^000000",
			"Armor Level:^777777 1^000000",
			"________________________",
			"Requirement:^777777 None^000000"
		},
		slotCount = 1,
		ClassNum = 17,
		costume = false
	},
}

-- Official items whose name, description or look you change: same format.
tbl_override = {
}
