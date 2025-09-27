use xcstrings_mcp::store::{TranslationUpdate, XcStringsStore};

#[tokio::test]
async fn test_preserves_apple_format() -> Result<(), Box<dyn std::error::Error>> {
    // Create a test file with Apple's specific formatting
    let test_dir = tempfile::tempdir()?;
    let test_file = test_dir.path().join("test.xcstrings");

    let apple_formatted_content = r#"{
  "version" : "1.0",
  "sourceLanguage" : "en",
  "strings" : {
    "greeting.morning" : {
      "extractionState" : "manual",
      "localizations" : {
        "en" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "Good morning"
          }
        },
        "es" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "Buenos días"
          }
        }
      }
    },
    "greeting.evening" : {
      "localizations" : {
        "en" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "Good evening"
          }
        }
      }
    }
  }
}"#;

    // Write the initial file
    tokio::fs::write(&test_file, apple_formatted_content).await?;

    // Load the store
    let store = XcStringsStore::load_or_create(&test_file).await?;

    // Make a minimal change - add a French translation
    let update =
        TranslationUpdate::from_value_state(Some("Bonjour".into()), Some("translated".into()));

    store
        .upsert_translation("greeting.morning", "fr", update)
        .await?;

    // Read the modified content
    let modified_content = tokio::fs::read_to_string(&test_file).await?;

    // Verify Apple format is preserved (spaces before colons)
    assert!(modified_content.contains("\"version\" : \"1.0\""));
    assert!(modified_content.contains("\"sourceLanguage\" : \"en\""));
    assert!(modified_content.contains("\"strings\" : {"));
    assert!(modified_content.contains("\"greeting.morning\" : {"));
    assert!(modified_content.contains("\"greeting.evening\" : {"));
    assert!(modified_content.contains("\"localizations\" : {"));
    assert!(modified_content.contains("\"stringUnit\" : {"));
    assert!(modified_content.contains("\"state\" : \"translated\""));
    assert!(modified_content.contains("\"value\" : \"Good morning\""));

    // Verify key order is preserved
    let morning_pos = modified_content.find("\"greeting.morning\"").unwrap();
    let evening_pos = modified_content.find("\"greeting.evening\"").unwrap();
    assert!(morning_pos < evening_pos, "Key order should be preserved");

    // Verify language order within localizations is preserved (en before es, fr added after)
    let en_content = "\"en\" : {";
    let es_content = "\"es\" : {";
    let fr_content = "\"fr\" : {";

    let en_pos = modified_content.find(en_content).unwrap();
    let es_pos = modified_content.find(es_content).unwrap();
    let fr_pos = modified_content.find(fr_content).unwrap();

    assert!(en_pos < es_pos, "English should come before Spanish");
    assert!(
        es_pos < fr_pos,
        "Spanish should come before French (newly added)"
    );

    // Verify the new French translation was added
    assert!(modified_content.contains("\"value\" : \"Bonjour\""));

    // Verify existing translations are unchanged
    assert!(modified_content.contains("\"value\" : \"Good morning\""));
    assert!(modified_content.contains("\"value\" : \"Buenos días\""));
    assert!(modified_content.contains("\"value\" : \"Good evening\""));

    Ok(())
}

#[tokio::test]
async fn test_preserves_key_order_on_update() -> Result<(), Box<dyn std::error::Error>> {
    let test_dir = tempfile::tempdir()?;
    let test_file = test_dir.path().join("order_test.xcstrings");

    let initial_content = r#"{
  "version" : "1.0",
  "sourceLanguage" : "en",
  "strings" : {
    "first" : {
      "localizations" : {
        "en" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "First"
          }
        }
      }
    },
    "second" : {
      "localizations" : {
        "en" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "Second"
          }
        }
      }
    },
    "third" : {
      "localizations" : {
        "en" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "Third"
          }
        }
      }
    }
  }
}"#;

    tokio::fs::write(&test_file, initial_content).await?;
    let store = XcStringsStore::load_or_create(&test_file).await?;

    // Update the middle key
    let update = TranslationUpdate::from_value_state(Some("Updated Second".into()), None);
    store.upsert_translation("second", "en", update).await?;

    // Add a new key - should be added at the end
    let new_update =
        TranslationUpdate::from_value_state(Some("Fourth".into()), Some("translated".into()));
    store.upsert_translation("fourth", "en", new_update).await?;

    let modified_content = tokio::fs::read_to_string(&test_file).await?;

    // Check order is preserved
    let first_pos = modified_content.find("\"first\"").unwrap();
    let second_pos = modified_content.find("\"second\"").unwrap();
    let third_pos = modified_content.find("\"third\"").unwrap();
    let fourth_pos = modified_content.find("\"fourth\"").unwrap();

    assert!(first_pos < second_pos, "first should come before second");
    assert!(second_pos < third_pos, "second should come before third");
    assert!(
        third_pos < fourth_pos,
        "third should come before fourth (newly added)"
    );

    // Verify updates were applied
    assert!(modified_content.contains("\"value\" : \"Updated Second\""));
    assert!(modified_content.contains("\"value\" : \"Fourth\""));

    Ok(())
}

#[tokio::test]
async fn test_preserves_format_with_variations() -> Result<(), Box<dyn std::error::Error>> {
    let test_dir = tempfile::tempdir()?;
    let test_file = test_dir.path().join("variations_test.xcstrings");

    let initial_content = r#"{
  "version" : "1.0",
  "sourceLanguage" : "en",
  "strings" : {
    "items.count" : {
      "localizations" : {
        "en" : {
          "variations" : {
            "plural" : {
              "one" : {
                "stringUnit" : {
                  "state" : "translated",
                  "value" : "%lld item"
                }
              },
              "other" : {
                "stringUnit" : {
                  "state" : "translated",
                  "value" : "%lld items"
                }
              }
            }
          }
        }
      }
    }
  }
}"#;

    tokio::fs::write(&test_file, initial_content).await?;
    let store = XcStringsStore::load_or_create(&test_file).await?;

    // Add a "zero" case to the plural variation
    let update = TranslationUpdate::from_value_state(None, None).add_variation(
        "plural",
        "zero",
        TranslationUpdate::from_value_state(Some("No items".into()), Some("translated".into())),
    );

    store
        .upsert_translation("items.count", "en", update)
        .await?;

    let modified_content = tokio::fs::read_to_string(&test_file).await?;

    // Verify format is preserved
    assert!(modified_content.contains("\"variations\" : {"));
    assert!(modified_content.contains("\"plural\" : {"));

    // Verify order within variations (one, other should come before zero)
    let one_pos = modified_content.find("\"one\" :").unwrap();
    let other_pos = modified_content.find("\"other\" :").unwrap();
    let zero_pos = modified_content.find("\"zero\" :").unwrap();

    assert!(one_pos < other_pos, "one should come before other");
    assert!(
        other_pos < zero_pos,
        "other should come before zero (newly added)"
    );

    // Verify content
    assert!(modified_content.contains("\"value\" : \"%lld item\""));
    assert!(modified_content.contains("\"value\" : \"%lld items\""));
    assert!(modified_content.contains("\"value\" : \"No items\""));

    Ok(())
}

#[tokio::test]
async fn test_preserves_stringunit_field_order() -> Result<(), Box<dyn std::error::Error>> {
    let test_dir = tempfile::tempdir()?;
    let test_file = test_dir.path().join("stringunit_order_test.xcstrings");

    // Create a file with Apple's specific stringUnit field order (state before value)
    let initial_content = r#"{
  "version" : "1.0",
  "sourceLanguage" : "en",
  "strings" : {
    "test.key" : {
      "comment" : "Test comment",
      "extractionState" : "manual",
      "localizations" : {
        "en" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "Original value"
          }
        },
        "fr" : {
          "stringUnit" : {
            "state" : "needs_review",
            "value" : "Valeur originale"
          }
        }
      }
    }
  }
}"#;

    tokio::fs::write(&test_file, initial_content).await?;
    let store = XcStringsStore::load_or_create(&test_file).await?;

    // Update just the English value (not the state)
    let update = TranslationUpdate::from_value_state(Some("Updated value".into()), None);
    store.upsert_translation("test.key", "en", update).await?;

    let modified_content = tokio::fs::read_to_string(&test_file).await?;

    // Find the English stringUnit section
    let en_unit_start = modified_content
        .find("\"en\" : {\n          \"stringUnit\" : {")
        .unwrap();
    let en_unit_end = modified_content[en_unit_start..]
        .find("          }")
        .unwrap()
        + en_unit_start;
    let en_unit = &modified_content[en_unit_start..en_unit_end];

    // Verify that "state" appears before "value" in the English stringUnit
    let state_pos = en_unit.find("\"state\"").expect("state field not found");
    let value_pos = en_unit.find("\"value\"").expect("value field not found");
    assert!(
        state_pos < value_pos,
        "In stringUnit, 'state' should come before 'value'"
    );

    // Verify the exact format matches Apple's style
    assert!(en_unit.contains("\"state\" : \"translated\""));
    assert!(en_unit.contains("\"value\" : \"Updated value\""));

    // Check that the French localization is unchanged and maintains order
    let fr_unit_start = modified_content
        .find("\"fr\" : {\n          \"stringUnit\" : {")
        .unwrap();
    let fr_unit_end = modified_content[fr_unit_start..]
        .find("          }")
        .unwrap()
        + fr_unit_start;
    let fr_unit = &modified_content[fr_unit_start..fr_unit_end];

    let fr_state_pos = fr_unit.find("\"state\"").expect("state field not found");
    let fr_value_pos = fr_unit.find("\"value\"").expect("value field not found");
    assert!(
        fr_state_pos < fr_value_pos,
        "In French stringUnit, 'state' should come before 'value'"
    );

    assert!(fr_unit.contains("\"state\" : \"needs_review\""));
    assert!(fr_unit.contains("\"value\" : \"Valeur originale\""));

    // Also verify that comment comes before extractionState in the entry
    let comment_pos = modified_content.find("\"comment\"").unwrap();
    let extraction_pos = modified_content.find("\"extractionState\"").unwrap();
    assert!(
        comment_pos < extraction_pos,
        "comment should come before extractionState"
    );

    Ok(())
}

#[tokio::test]
async fn test_preserves_version_key_position() -> Result<(), Box<dyn std::error::Error>> {
    let test_dir = tempfile::tempdir()?;

    // Test 1: Version at the end (after strings)
    let test_file_end = test_dir.path().join("version_at_end.xcstrings");
    let content_version_at_end = r#"{
  "sourceLanguage" : "en",
  "strings" : {
    "greeting" : {
      "localizations" : {
        "en" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "Hello"
          }
        }
      }
    }
  },
  "version" : "1.0"
}"#;

    tokio::fs::write(&test_file_end, content_version_at_end).await?;
    let store = XcStringsStore::load_or_create(&test_file_end).await?;

    // Make a change
    let update = TranslationUpdate::from_value_state(Some("Hi".into()), None);
    store.upsert_translation("greeting", "en", update).await?;

    let result = tokio::fs::read_to_string(&test_file_end).await?;

    // Verify version is still at the end
    let version_pos = result.find("\"version\"").expect("version not found");
    let strings_pos = result.find("\"strings\"").expect("strings not found");
    assert!(
        version_pos > strings_pos,
        "Version should remain at the end after strings"
    );

    // Test 2: Version in the middle
    let test_file_middle = test_dir.path().join("version_in_middle.xcstrings");
    let content_version_middle = r#"{
  "sourceLanguage" : "en",
  "version" : "1.0",
  "strings" : {
    "greeting" : {
      "localizations" : {
        "en" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "Hello"
          }
        }
      }
    }
  }
}"#;

    tokio::fs::write(&test_file_middle, content_version_middle).await?;
    let store = XcStringsStore::load_or_create(&test_file_middle).await?;

    // Make a change
    let update = TranslationUpdate::from_value_state(Some("Hi".into()), None);
    store.upsert_translation("greeting", "en", update).await?;

    let result = tokio::fs::read_to_string(&test_file_middle).await?;

    // Verify version is still in the middle
    let source_pos = result
        .find("\"sourceLanguage\"")
        .expect("sourceLanguage not found");
    let version_pos = result.find("\"version\"").expect("version not found");
    let strings_pos = result.find("\"strings\"").expect("strings not found");

    assert!(
        source_pos < version_pos && version_pos < strings_pos,
        "Version should remain in the middle between sourceLanguage and strings"
    );

    // Test 3: Version at the beginning
    let test_file_begin = test_dir.path().join("version_at_beginning.xcstrings");
    let content_version_begin = r#"{
  "version" : "1.0",
  "sourceLanguage" : "en",
  "strings" : {
    "greeting" : {
      "localizations" : {
        "en" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "Hello"
          }
        }
      }
    }
  }
}"#;

    tokio::fs::write(&test_file_begin, content_version_begin).await?;
    let store = XcStringsStore::load_or_create(&test_file_begin).await?;

    // Make a change
    let update = TranslationUpdate::from_value_state(Some("Hi".into()), None);
    store.upsert_translation("greeting", "en", update).await?;

    let result = tokio::fs::read_to_string(&test_file_begin).await?;

    // Verify version is still at the beginning
    let version_pos = result.find("\"version\"").expect("version not found");
    let source_pos = result
        .find("\"sourceLanguage\"")
        .expect("sourceLanguage not found");

    assert!(
        version_pos < source_pos,
        "Version should remain at the beginning before sourceLanguage"
    );

    Ok(())
}
