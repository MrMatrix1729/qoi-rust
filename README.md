# qoi-rust

This is a QOI decoder made in rust. QOI is the quite OK format known for it's fast encoding and decoding specs. 

## How to use
To decode an qoi image to rust, just run
```
cargo run -- /path/to/image.qoi
```

## References
- https://qoiformat.org/

## Future prospects
- Making the code run in parallel, by exploiting the `prev_pixel`

