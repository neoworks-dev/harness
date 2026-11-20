# training

A plugin that shows what surfaces are for: a training run you watch and steer while it runs, from neo's window and terminal.

- **A bar over the input** (`surfaces/training.neoui`, `bottom`): the run's state, epoch, progress, the loss as a sparkline, learning rate and time left, with Pause, Resume and Start.
- **A page** (`surfaces/training-vars.neoui`, `page`): the loss over the run as a chart, the model so far, and the learning rate, epochs and pause to change while it runs.
- **Their data** comes from an MCP server the plugin brings (`server.py`, connector `mcp-training`): `status`, `set` and `start`. The run itself is `train.py`, a small gradient descent slow enough to watch, which writes its status to a file and reads its controls from another every few steps. Neither needs anything but Python 3.

A real run (PyTorch, TensorFlow) would do the same in its training loop: write the numbers you want to see, read the knobs you want to turn.

## Try it

```
/plugin marketplace add <this repository>/examples
/plugin install training@neo-examples
/surface training
/surface training-vars
```

Press **Start a run** in the bar. The agent can open them too (`ui_surface {action: "open", name: "training"}`), or write a surface of its own for whatever you are running.

The run's files are in `TRAINING_DIR` (default `/tmp/neo-training`).
