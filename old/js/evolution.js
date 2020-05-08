let NN = new Network(2, [6], 2);

console.log(NN)

let input = [1,2]

output = NN.think(input)

let max = 0;
for (let i = 1; i < output.length; ++i) {
    if (output[i] > output[max]) max = i
}
console.log(output, output[max])
