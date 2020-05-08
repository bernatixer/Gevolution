// TODO: Activation function
class Network {
    constructor(inputs, hiddens, outputs) {
        this.input = [];
        this.hiddens = [];
        let size = inputs;
        for (let i = 0; i < hiddens.length; ++i) {
            this.hiddens[i] = new Layer(hiddens[i], size);
            size = hiddens[i];
        }
        this.output = new Layer(outputs, size);
    }

    think(input) {
        let output = input;
        for (let i = 0; i < this.hiddens.length; ++i) {
            output = this.hiddens[i].getLayer(output);
        }
        return output;
    }
}

class Layer {
    constructor(size, last) {
        this.perceptrons = [];
        for (let i = 0; i < size; ++i) {
            this.perceptrons[i] = new Perceptron(last);
        }
    }
    
    getLayer(last) {
        let layer = [];
        for (let i = 0; i < this.perceptrons.length; ++i) {
            layer[i] = this.perceptrons[i].compute(last);
        }
        return layer;
    }
}

class Perceptron {
    constructor(size) {
        this.bias = Math.random()*2 - 1;
        this.weights = [];
        for (let i = 0; i < size; ++i) {
            this.weights[i] = Math.random()*2 - 1;
        }
    }

    compute(last) {
        console.log(last.length, this.weights.length)
        let sum = 0;
        for (let i = 0; i < last.length; ++i) {
            sum += last[i]*this.weights[i];
        }
        return sum - this.bias;
    }
}
