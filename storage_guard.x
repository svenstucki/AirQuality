/* The application image must stay below the upper-half flash history area. */
ASSERT(_rodata_end < ORIGIN(ROTEXT) + 0x00200000,
       "AirQuality firmware overlaps the persistent storage region");
