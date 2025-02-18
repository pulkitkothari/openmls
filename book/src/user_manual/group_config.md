# Group configuration

The group configuration can be specified by building a `MlsGroupConfig` object or choosing the default value. The default velaue contains safe values for all parameters and is suitable for scenarios whithout particular requirements.

The following parameters can be set:

| Name                           | Type                            | Explanation                                                                                      |
| ------------------------------ | ------------------------------- | ------------------------------------------------------------------------------------------------ |
| `wire_format`                  | `WireFormat`                    | Defines whether handshake messages are encrypted. The default `Ciphertext`.                      |
| `padding_size`                 | `usize`                         | Size of padding in bytes. The default is 0.                                                      |
| `max_past_epochs`              | `usize`                         | Maximum number of past epochs for which application messages can be decrypted. The default is 0. |
| `number_of_resumption_secrets` | `usize`                         | Number of resumtion secrets to keep. The default is 0.                                           |
| `use_ratchet_tree_extension`   | `bool`                          | Flag indicating the Ratchet Tree Extension should be used. The default is `false`.               |
| `required_capabilities`        | `RequiredCapabilitiesExtension` | Required capabilities (extensions and proposal types).                                           |
| `sender_ratchet_configuration` | `SenderRatchetConfiguration`    | Sender ratchet configuration.                                                                    |

Example configuration:

```rust,no_run,noplayground
{{#include ../../../openmls/tests/book_code.rs:mls_group_config_example}}
```
