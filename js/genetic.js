let INITIAL_POPULATION = 100;
let GENES_SIZE = 512;
let FITTEST = 20;
let MUTATE_PROB = 1/GENES_SIZE;
let CROSSOVER = 0.5;
let MAX_GENERATIONS = 100;

let population = [];

class Individual {
    constructor(genes) {
      this.genes = genes;
    }
}

function createInitialPopulation() {
    for (let i=0; i<INITIAL_POPULATION; ++i) {
        let genes = new Uint8Array(GENES_SIZE);
        for (let g=0; g<GENES_SIZE; ++g) genes[g] = (Math.round(Math.random()));
        let ind = new Individual(genes);
        population.push(ind);
    }
}

function fitness(ind) {
    let sum = 0;
    for (let g=0; g<GENES_SIZE; ++g) sum += ind.genes[g];
    return sum;
}

function selection() {
    population.sort(function(a, b) {
        return fitness(b) - fitness(a);
    });
    population = population.slice(0, FITTEST);
}

function mutate(genes) {
    for (let g=0; g<GENES_SIZE; ++g) {
        if (Math.random() < MUTATE_PROB) {
            if (genes[g]) genes[g] = 0;
            else genes[g] = 1;
        }
    }
    return genes;
}

function generateOffspring() {
    let offspring = []
    let size = population.length
    for (let f=0; f<size; ++f) {
        for (let m=f+1; m<size; ++m) {
            let newGenes = new Uint8Array(GENES_SIZE);
            for (let i=0; i<GENES_SIZE; ++i) {
                if (Math.random() < CROSSOVER) newGenes[i] = population[f].genes[i];
                else newGenes[i] = population[m].genes[i];
            }
            let newInd = new Individual(mutate(newGenes));
            offspring.push(newInd);
        }
    }
    population = offspring;
}

createInitialPopulation();

for (let i=0; i<MAX_GENERATIONS; ++i) {
    let fittest = fitness(population[0]);
    for (let p=0; p<population.length; ++p) {
        if (fitness(population[p]) > fittest) fittest = fitness(population[p]);
    }
    console.log('GENERATION: ' + (i+1));
    console.log('POPULATION: ' + population.length);
    console.log('FITTEST: ' + fittest);
    console.log('-------------------');
    if (fittest == GENES_SIZE) {
        console.log('FOUND FITTEST AT GENERATION ' + (i+1));
        break;
    }
    selection();
    generateOffspring();
}
