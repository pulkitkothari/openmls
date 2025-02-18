# Creating application messages

Application messages are created from byte slices with the `.create_message()` function:

```rust,no_run,noplayground
{{#include ../../../openmls/tests/book_code.rs:create_application_message}}
```

Note that the theoretical maximum length of application messages is 2^32 bytes, however in practice messages should be much shorter unless the Delivery Service can cope with very long messages.

The function returns an `MlsMessageOut` that needs to be sent to the Delivery Service for fanout to other members of the group. To guarantee the best possible Forward Secrecy, the key material used to encrypt messages is immediately discarded after encryption. This means that the message author cannot decrypt application messages. If access to the messages content is required after creating the message, a copy of the plaintext message should be kept by the application.
