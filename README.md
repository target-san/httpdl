# httpdl project

A small demo utility which demonstrates implementation of simple
HTTP/HTTPS file downoader

* Download is performed using async I/O, namely tokio runtime and reqwest HTTP client
* User can specify number of files downloaded concurrently and global download speed limit
* Code is covered with unit tests, not thoroughly but enough to demonstrate
    testing of async code and use of stub web server for integration test purposes
