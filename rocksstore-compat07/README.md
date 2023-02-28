# openraft-rockcsstore-0708

This is an example `RaftStorage` implementation illustrating how to upgrade an application based on [openraft-0.7](https://github.com/datafuselabs/openraft/tree/release-0.7) to [openraft-0.8](https://github.com/datafuselabs/openraft/tree/release-0.8).

In this example, it assumes there is an on-disk rocksdb dir that backs an 0.7 application,
and the compatible 0708 should be able to read data from this dir and write data to it.
